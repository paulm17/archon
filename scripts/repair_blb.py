"""Apply a prepare-blb-repair export to an existing Archon scripture database.

Preview by default. Pass --apply to commit the same checked changes.
"""
import argparse
from collections import Counter
from itertools import zip_longest
import json
from pathlib import Path
import sqlite3
import time
import os
import tempfile


def require(condition, message):
    if not condition:
        raise ValueError(message)


def uid_bytes(uid):
    return int(uid).to_bytes(8, "big")


def data_key(uid, field):
    return b"\x01" + uid_bytes(uid) + field.encode()


def field_value(db, uid, field):
    row = db.execute("SELECT value FROM archondb_main WHERE key=?", (data_key(uid, field),)).fetchone()
    return json.loads(row[0][16:]) if row else None


def children(db, uid, field):
    prefix = b"\x07" + uid_bytes(uid) + field.encode()
    return [int.from_bytes(row[0][-8:], "big") for row in db.execute(
        "SELECT key FROM archondb_main WHERE key>=? AND key<?", (prefix + b"\0", prefix + b"\x01"))]


def resolve_ids(db):
    key = b'\x02Translations.code\0"BLB"'
    blb = db.execute("SELECT value FROM archondb_main WHERE key=?", (key,)).fetchone()
    require(blb, "BLB is missing from the target database")
    translation = int.from_bytes(blb[0], "big")
    ids = {}
    for edition in children(db, translation, "bookTranslations"):
        book = field_value(db, field_value(db, edition, "book"), "code")
        for chapter in children(db, edition, "chapters"):
            number = field_value(db, chapter, "number")
            for verse in children(db, chapter, "verses"):
                suffix = f"{book}::BLB::{number}::{field_value(db, verse, 'number')}"
                key = "verse::" + suffix
                require(key not in ids, f"Duplicate {key}")
                ids[key] = verse
                contents = children(db, verse, "verseContents")
                require(len(contents) == 1, f"Unexpected content count for {key}")
                ids["verse-content::" + suffix] = contents[0]
    return ids


def order_key(kind, field, uid, value, descending):
    raw = value.encode()
    if descending:
        raw = bytes(b ^ 255 for b in raw)
    return b"\x09" + kind.encode() + b"\0" + field.encode() + b"\0" + bytes([descending]) + raw + b"\0" + uid_bytes(uid)


def replace_field(db, kind, uid, field, before, after, stamp):
    key = data_key(uid, field)
    current = db.execute("SELECT ts FROM archondb_main WHERE key=?", (key,)).fetchone()
    if current:
        require(current[0] < stamp, f"Newer timestamp at {uid}.{field}")
    raw = json.dumps(after, ensure_ascii=False).encode()
    db.execute("INSERT OR REPLACE INTO archondb_main VALUES(?,?,?)", (key, stamp + raw, stamp))
    db.execute("INSERT INTO archondb_history VALUES(?,?)", (stamp + key[1:], raw))
    for descending in (False, True):
        if before is not None:
            db.execute("DELETE FROM archondb_main WHERE key=?", (order_key(kind, field, uid, before, descending),))
        if after is not None:
            db.execute("INSERT OR REPLACE INTO archondb_main VALUES(?,?,?)",
                       (order_key(kind, field, uid, after, descending), b"", stamp))


def search_row(db, uid, expected):
    scoped = db.execute("SELECT rowid,text_content FROM fts_scope_data WHERE uid MATCH ? AND field='text.fulltext'",
                        (f"u{uid}",)).fetchall()
    require(len(scoped) == 1 and scoped[0][1] == expected, f"Scoped search mismatch for {uid}")
    rowid = scoped[0][0]
    native = db.execute("SELECT uid,field,text_content FROM fts_data WHERE rowid=?", (rowid,)).fetchone()
    require(native == (uid, "text.fulltext", expected), f"Search mismatch for {uid}")
    return rowid


def repair(stage, database, apply):
    mode = "rw" if apply else "ro"
    with sqlite3.connect(database.resolve().as_uri() + "?mode=" + mode, uri=True) as db:
        db.execute("BEGIN IMMEDIATE" if apply else "BEGIN")
        ids = resolve_ids(db)
        stamp = (time.time_ns() // 1_000_000).to_bytes(6, "big") + bytes(10)
        changes = []
        totals = Counter()
        for filename, kind, fields in [
            ("verses", "Verse", {"metadata": "metadata", "chapter_heading": "chapterHeading"}),
            ("verse_contents", "VerseContent", {"text": "text"}),
            ("chunks", "Chunk", {"text": "text"}),
        ]:
            name = filename + ".jsonl"
            with (stage / "before" / name).open() as old, (stage / "after" / name).open() as new:
                for left, right in zip_longest(old, new):
                    require(left and right, f"Record count changed in {name}")
                    before, after = json.loads(left), json.loads(right)
                    key = before["id"]
                    require(key == after["id"], f"Record mismatch at {key}")
                    require({k: v for k, v in before.items() if k not in fields} ==
                            {k: v for k, v in after.items() if k not in fields}, f"Unexpected structure change at {key}")
                    uid = ids.get(key)
                    if kind == "Chunk":
                        start, end = ids[before["start_verse_id"]], ids[before["end_verse_id"]]
                        matches = [c for c in children(db, start, "chunks")
                                   if int(field_value(db, c, "startVerse")) == start and int(field_value(db, c, "endVerse")) == end]
                        require(len(matches) == 1, f"Ambiguous chunk at {key}")
                        uid = matches[0]
                    require(uid is not None, f"Missing {key}")
                    for prop, field in fields.items():
                        old_value, new_value = before.get(prop), after.get(prop)
                        actual = field_value(db, uid, field)
                        if actual == new_value:
                            if field == "text" and old_value != new_value:
                                search_row(db, uid, actual)
                            continue
                        require(actual == old_value, f"Source mismatch at {key}.{field}")
                        changes.append((kind, uid, field, actual, new_value))
                        totals[(kind, field)] += 1
        print("Changed fields:", dict(totals), flush=True)
        for kind, uid, field, old, new in changes:
            if field == "text":
                rowid = search_row(db, uid, old)
                if apply:
                    for table in ("fts_data", "fts_scope_data"):
                        db.execute(f"UPDATE {table} SET text_content=? WHERE rowid=?", (new, rowid))
            if apply:
                replace_field(db, kind, uid, field, old, new, stamp)
        return len(changes)


def check_chapter_files(stage, root):
    # Only publish BLB chapters that still match the staged input. The same
    # stage can safely finish publishing after an interruption.
    original = stage / "legacy/en/BLB"
    rebuilt = stage / "json/en/BLB"
    live = root / "en/BLB"
    files = {p.relative_to(original) for p in original.glob("*/chapter_*.json")}
    for tree in (rebuilt, live):
        require(files == {p.relative_to(tree) for p in tree.glob("*/chapter_*.json")},
                f"Chapter set differs at {tree}")
    for name in sorted(files):
        current = (live / name).read_bytes()
        updated = (rebuilt / name).read_bytes()
        if current != updated:
            require(current == (original / name).read_bytes(), f"Chapter changed at {name}")
            yield live / name, current, updated


def publish_chapters(stage, root):
    count = 0
    for path, _, content in check_chapter_files(stage, root):
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temp:
            temp.write(content)
            temp.flush()
            os.fsync(temp.fileno())
        os.chmod(temp.name, path.stat().st_mode)
        os.replace(temp.name, path)
        count += 1
    print(f"{count} rebuilt BLB chapter files published")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", type=Path)
    parser.add_argument("database", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--chapter-root", type=Path, help="Existing chapter JSON directory to verify and publish")
    args = parser.parse_args()
    if args.chapter_root:
        print("Changed chapter files:", sum(1 for _ in check_chapter_files(args.stage, args.chapter_root)))
    started = time.monotonic()
    count = repair(args.stage, args.database, args.apply)
    if args.apply and args.chapter_root:
        publish_chapters(args.stage, args.chapter_root)
    print(f"{count} field changes {'committed' if args.apply else 'previewed'} in {time.monotonic() - started:.1f}s")
