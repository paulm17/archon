//! End-to-end offline check: saved HTML in, JSON out.
//!
//! No network, no browser — it exercises the same code path as `scraper json`
//! against a throwaway output tree.

use std::path::PathBuf;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "scraper_unified_{tag}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn write(path: PathBuf, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

const GATEWAY: &str = r#"<html><body><select><option selected="selected" value="NIV">NIV</option></select><div class="passage-text"><div class="passage-content"><div class="std-text">
<h3><span class="text Gen-1-1">The Beginning</span></h3>
<p class="hang-2 chapter-1"><span class="text Gen-1-1"><span class="chapternum">1&nbsp;</span>In the beginning God created the heavens and the earth.</span></p>
<p class="hang-2"><span class="text Gen-1-2">And the earth was formless and empty.</span></p>
</div></div></div></body></html>"#;

/// One document holding two chapters of Luke, as the OEB really delivers it.
const OEB_BOOK: &str = r##"<html><body>
<h1>Mark</h1><p class="c-num">[Mark 16]</p><p><span class="v-num">[1]</span> End of Mark.</p>
<a href="#3">Luke</a><h1 id="3"></h1><h1>Luke</h1>
<p class="c-num">[Luke 1]</p>
<h5>Dedication</h5>
<p> <span class="v-num">[1]</span>  To his Excellency, Theophilus. </p>
<p>  Many attempts have been made,  <span class="v-num">[2]</span>  just as they were reported to us. </p>
<p class="c-num">[Luke 2]</p>
<p> <span class="v-num">[1]</span> In those days a decree went out from Caesar Augustus. </p>
</body></html>"##;

#[test]
fn json_is_produced_from_saved_html() {
    let root = temp_dir("workflow");
    write(root.join("gospels/en/NIV/GEN/chapter_1.html"), GATEWAY);
    write(root.join("gospels/en/Luke/OEB/chapter_1.html"), OEB_BOOK);
    write(
        root.join("gospels/en/Mark/NIV/chapter_1.html"),
        &GATEWAY.replace("Gen-", "Mark-"),
    );
    write(
        root.join("gospels/en/PrAz/NRSVA/chapter_1.html"),
        &GATEWAY.replace("Gen-", "PrAzar-"),
    );

    let outcome = archon_json::run(&root, &[]).expect("json run");
    assert!(outcome.failed.is_empty(), "failures: {:?}", outcome.failed);
    assert_eq!(outcome.converted, 5);
    assert!(root.join("json/en/NIV/MRK/chapter_1.json").is_file());
    let alias = std::fs::read(root.join("json/en/NRSVA/S3Y/chapter_1.json")).unwrap();
    assert!(
        !serde_json::from_slice::<serde_json::Value>(&alias).unwrap()["verses"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let niv = root.join("json/en/NIV/GEN/chapter_1.json");
    let text = std::fs::read_to_string(&niv).expect("NIV json");
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["verses"][0]["verse"], 1);
    assert_eq!(parsed["book_code"], "Gen");
    assert_eq!(parsed["translation_code"], "NIV");

    // The whole-book document was sliced into per-chapter JSON.
    let luke2 = root.join("json/en/OEB/LUK/chapter_2.json");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&luke2).expect("Luke 2 json")).unwrap();
    assert_eq!(parsed["verses"][0]["verse"], 1);
    assert!(
        parsed["verses"][0]["content"]
            .to_string()
            .contains("decree went out"),
        "chapter 2 should hold its own verses, not chapter 1's"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn extracts_blb_and_cpdv_without_touching_existing_files() {
    let root = temp_dir("blb_cpdv");
    let mut blb = String::from(
        r##"
        <title>Luke 10 Berean Literal Bible</title>
        <div id="leftbox"><div class="grk">
        <p class="hdg">A section heading<br><span class="cross">(Matthew 1:1)</span>
        <p class="reg">
    "##,
    );
    let mut cpdv = String::from(
        r##"
        <title>Luke - Catholic Public Domain Version of the Sacred Bible</title>
        <a class=title href="NT-03_Luke.htm">The Gospel of Luke</a>
        <font size=4>[<a class=chapter href="#top">Luke 9</a>]<br>{P:1} Prologue.<br>{9:1} Previous chapter.<br>
        [<a class=chapter href="#top">Luke 10</a>]<br>
    "##,
    );
    for verse in 1..=42 {
        blb.push_str(&format!(
            r#"<span class="reftext"><a href="/luke/10-{verse}.htm"><b>{verse}</b></a></span>BLB verse {verse}."#
        ));
        if verse == 1 {
            blb.push_str(r##" Given<span class="fn"><a href="#fn">†</a></span> to you⁺<span class="fn"><a href="#fn">†</a></span>."##);
        }
        cpdv.push_str(&format!("{{10:{verse}}} CPDV verse {verse}.<br>\n"));
    }
    blb.push_str(
        r#"<p><a name="fn"></a><br><b>Footnotes:</b><br>
        <span class="fnverse">1</span> A <i>footnote</i>.<br><span class="fnverse">1</span> Second note.<br></div></div>"#,
    );
    cpdv.push_str(
        r##"[<a class=chapter href="#top">Luke 11</a>]<br>{11:1} Next chapter.<br></font>"##,
    );
    let earlier = temp_dir("blb_cpdv_earlier_layout");
    let earlier_blb = "gospels/en/BLB/LUK/chapter_10.html";
    let earlier_cpdv = "gospels/en/CPDV/LUK/book.html";
    write(earlier.join(earlier_blb), &blb);
    write(earlier.join(earlier_cpdv), &cpdv);

    write(root.join("gospels/en/Luke/BLB/chapter_10.html"), &blb);
    write(root.join("gospels/en/Luke/CPDV/book.html"), &cpdv);
    let alternate_blb = blb.replace("BLB verse", "Alternate BLB verse");
    let alternate_cpdv = cpdv.replace("CPDV verse", "Alternate CPDV verse");
    write(root.join(earlier_blb), &alternate_blb);
    write(root.join(earlier_cpdv), &alternate_cpdv);
    write(root.join("gospels/en/NIV/GEN/chapter_1.html"), GATEWAY);
    let sentinel = root.join("json/en/NIV/GEN/chapter_1.json");
    write(sentinel.clone(), "existing output");

    let translations = &["blb", "CPDV"];
    let earlier_outcome = archon_json::run(&earlier, translations).unwrap();
    assert!(
        earlier_outcome.failed.is_empty(),
        "{:?}",
        earlier_outcome.failed
    );
    assert_eq!(earlier_outcome.converted, 4);
    for code in ["BLB", "CPDV"] {
        assert!(
            earlier
                .join(format!("json/en/{code}/LUK/chapter_10.json"))
                .is_file()
        );
    }
    let outcome = archon_json::run(&root, translations).unwrap();
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    assert_eq!(outcome.converted, 4);
    let chapter = |code, chapter| {
        let path = root.join(format!("json/en/{code}/LUK/chapter_{chapter}.json"));
        serde_json::from_slice::<archon_pipeline::model::ChapterData>(&std::fs::read(path).unwrap())
            .unwrap()
    };
    for code in ["BLB", "CPDV"] {
        let data = chapter(code, 10);
        assert_eq!(data.translation_code, code);
        assert_eq!(data.book_name, "Luke");
        assert_eq!(data.chapter, 10);
        assert_eq!(data.verses.len(), 42);
        assert_eq!(data.verses[41].verse, 42);
        assert_eq!(
            data.verses[41].content[0].text,
            Some(format!("{code} verse 42."))
        );
    }
    let blb = chapter("BLB", 10);
    assert_eq!(
        blb.verses[0].header[0].text.as_deref(),
        Some("A section heading")
    );
    assert_eq!(
        blb.verses[0].content[0].text.as_deref(),
        Some("BLB verse 1. Given to you.")
    );
    assert_eq!(blb.footnotes[0].text, "A footnote.");
    assert_eq!(blb.footnotes[1].text, "Second note.");
    let items = &blb.verses[0].content;
    assert_eq!(items[1].text.as_deref(), Some("†"));
    assert_eq!(items[1].number, Some(18));
    assert_eq!(items[2].kind, "plural");
    assert_eq!(items[2].number, Some(25));
    assert_eq!(items[3].number, Some(25));
    assert_eq!(items[3].id.as_ref(), Some(&blb.footnotes[1].id));
    assert_eq!(
        blb.verses[0].content[1].id.as_ref(),
        Some(&blb.footnotes[0].id)
    );
    assert_eq!(
        chapter("CPDV", 9).verses[0].content[0].text.as_deref(),
        Some("Previous chapter.")
    );
    assert_eq!(
        chapter("CPDV", 11).verses[0].content[0].text.as_deref(),
        Some("Next chapter.")
    );

    let rerun = archon_json::run(&root, translations).unwrap();
    assert!(rerun.failed.is_empty(), "{:?}", rerun.failed);
    assert_eq!((rerun.converted, rerun.skipped), (0, 4));
    assert_eq!(
        std::fs::read_to_string(sentinel).unwrap(),
        "existing output"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(earlier_blb)).unwrap(),
        alternate_blb
    );
    assert_eq!(
        std::fs::read_to_string(root.join(earlier_cpdv)).unwrap(),
        alternate_cpdv
    );
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&earlier).unwrap();
}
