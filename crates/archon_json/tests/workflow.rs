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
