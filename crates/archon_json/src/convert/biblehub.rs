use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;


#[derive(Debug, Serialize, Deserialize, Clone)]
struct InterlinearWord {
    strongs: String,
    reference: String,
    transliteration: String,
    original: String,
    english: String,
    grammar: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InterlinearVerse {
    chapter: u32,
    verse: u32,
    header: String,
    words: Vec<InterlinearWord>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CrossRefLink {
    book: String,
    chapter: u32,
    verse_start: u32,
    verse_end: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CrossRef {
    book: String,
    chapter: u32,
    verse: u32,
    links: Vec<CrossRefLink>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InterlinearChapterData {
    translation_code: String,
    translation_name: String,
    book_code: String,
    book_name: String,
    chapter: u32,
    verses: Vec<InterlinearVerse>,
    crossrefs: Vec<CrossRef>,
}

fn extract_book_name(document: &Html) -> Option<String> {
    let title_selector = Selector::parse("title").unwrap();

    if let Some(title) = document.select(&title_selector).next() {
        let title_text = title.text().collect::<String>();
        // Extract "Genesis" from "Genesis 2 Interlinear Bible"
        let re = Regex::new(r"^(\w+)\s+\d+\s+Interlinear Bible$").unwrap();
        if let Some(caps) = re.captures(&title_text) {
            return Some(caps[1].to_string());
        }
    }

    None
}

fn extract_chapter_number(document: &Html) -> Option<u32> {
    let title_selector = Selector::parse("title").unwrap();

    if let Some(title) = document.select(&title_selector).next() {
        let title_text = title.text().collect::<String>();
        // Extract "2" from "Genesis 2 Interlinear Bible"
        let re = Regex::new(r"^\w+\s+(\d+)\s+Interlinear Bible$").unwrap();
        if let Some(caps) = re.captures(&title_text) {
            if let Ok(chapter) = caps[1].parse::<u32>() {
                return Some(chapter);
            }
        }
    }

    None
}

fn parse_crossref(text: &str) -> Option<CrossRefLink> {
    // Parse formats like "Exodus 16:22-36" or "Hebrews 4:1-11"
    let re = Regex::new(r"^([^0-9]+)\s+(\d+):(\d+)(?:-(\d+))?$").unwrap();

    if let Some(caps) = re.captures(text.trim()) {
        let book = caps[1].trim().to_string();
        let chapter = caps[2].parse::<u32>().ok()?;
        let verse_start = caps[3].parse::<u32>().ok()?;
        let verse_end = caps.get(4).and_then(|m| m.as_str().parse::<u32>().ok());

        return Some(CrossRefLink {
            book,
            chapter,
            verse_start,
            verse_end,
        });
    }

    None
}

fn is_old_testament(book_name: &str) -> bool {
    let old_testament_books = [
        "Gen", "Genesis",
        "Exod", "Exodus",
        "Lev", "Leviticus",
        "Num", "Numbers",
        "Deut", "Deuteronomy",
        "Josh", "Joshua",
        "Judg", "Judges",
        "Ruth", "Ruth",
        "1Sam", "1 Samuel",
        "2Sam", "2 Samuel",
        "1Kgs", "1 Kings",
        "2Kgs", "2 Kings",
        "1Chr", "1 Chronicles",
        "2Chr", "2 Chronicles",
        "Ezra", "Ezra",
        "Neh", "Nehemiah",
        "Esth", "Esther",
        "Job", "Job",
        "Ps", "Psalms",
        "Prov", "Proverbs",
        "Eccl", "Ecclesiastes",
        "Song", "Song of Solomon",
        "Isa", "Isaiah",
        "Jer", "Jeremiah",
        "Lam", "Lamentations",
        "Ezek", "Ezekiel",
        "Dan", "Daniel",
        "Hos", "Hosea",
        "Joel", "Joel",
        "Amos", "Amos",
        "Obad", "Obadiah",
        "Jonah", "Jonah",
        "Mic", "Micah",
        "Nah", "Nahum",
        "Hab", "Habakkuk",
        "Zeph", "Zephaniah",
        "Hag", "Haggai",
        "Zech", "Zechariah",
        "Mal", "Malachi"
    ];

    old_testament_books.contains(&book_name)
}

fn is_new_testament(book_name: &str) -> bool {
    let new_testament_books = [
        "Matt", "Matthew",
        "Mark", "Mark",
        "Luke", "Luke",
        "John", "John",
        "Acts", "Acts",
        "Rom", "Romans",
        "1Cor", "1 Corinthians",
        "2Cor", "2 Corinthians",
        "Gal", "Galatians",
        "Eph", "Ephesians",
        "Phil", "Philippians",
        "Col", "Colossians",
        "1Thess", "1 Thessalonians",
        "2Thess", "2 Thessalonians",
        "1Tim", "1 Timothy",
        "2Tim", "2 Timothy",
        "Titus", "Titus",
        "Phlm", "Philemon",
        "Heb", "Hebrews",
        "Jas", "James",
        "1Pet", "1 Peter",
        "2Pet", "2 Peter",
        "1John", "1 John",
        "2John", "2 John",
        "3John", "3 John",
        "Jude", "Jude",
        "Rev", "Revelation"
    ];

    new_testament_books.contains(&book_name)
}

fn normalize_book_code(book_name: &str) -> String {
    match book_name {
        "Gen" | "Genesis" => "Gen",
        "Exod" | "Exodus" => "Exod",
        "Lev" | "Leviticus" => "Lev",
        "Num" | "Numbers" => "Num",
        "Deut" | "Deuteronomy" => "Deut",
        "Josh" | "Joshua" => "Josh",
        "Judg" | "Judges" => "Judg",
        "Ruth" => "Ruth",
        "1Sam" | "1 Samuel" => "1Sam",
        "2Sam" | "2 Samuel" => "2Sam",
        "1Kgs" | "1 Kings" => "1Kgs",
        "2Kgs" | "2 Kings" => "2Kgs",
        "1Chr" | "1 Chronicles" => "1Chr",
        "2Chr" | "2 Chronicles" => "2Chr",
        "Ezra" => "Ezra",
        "Neh" | "Nehemiah" => "Neh",
        "Esth" | "Esther" => "Esth",
        "Job" => "Job",
        "Ps" | "Psalms" | "Psalm" => "Ps",
        "Prov" | "Proverbs" => "Prov",
        "Eccl" | "Ecclesiastes" => "Eccl",
        "Song" | "Song of Solomon" => "Song",
        "Isa" | "Isaiah" => "Isa",
        "Jer" | "Jeremiah" => "Jer",
        "Lam" | "Lamentations" => "Lam",
        "Ezek" | "Ezekiel" => "Ezek",
        "Dan" | "Daniel" => "Dan",
        "Hos" | "Hosea" => "Hos",
        "Joel" => "Joel",
        "Amos" => "Amos",
        "Obad" | "Obadiah" => "Obad",
        "Jonah" => "Jonah",
        "Mic" | "Micah" => "Mic",
        "Nah" | "Nahum" => "Nah",
        "Hab" | "Habakkuk" => "Hab",
        "Zeph" | "Zephaniah" => "Zeph",
        "Hag" | "Haggai" => "Hag",
        "Zech" | "Zechariah" => "Zech",
        "Mal" | "Malachi" => "Mal",
        "Matt" | "Matthew" => "Matt",
        "Mark" => "Mark",
        "Luke" => "Luke",
        "John" => "John",
        "Acts" => "Acts",
        "Rom" | "Romans" => "Rom",
        "1Cor" | "1 Corinthians" => "1Cor",
        "2Cor" | "2 Corinthians" => "2Cor",
        "Gal" | "Galatians" => "Gal",
        "Eph" | "Ephesians" => "Eph",
        "Phil" | "Philippians" => "Phil",
        "Col" | "Colossians" => "Col",
        "1Thess" | "1 Thessalonians" => "1Thess",
        "2Thess" | "2 Thessalonians" => "2Thess",
        "1Tim" | "1 Timothy" => "1Tim",
        "2Tim" | "2 Timothy" => "2Tim",
        "Titus" => "Titus",
        "Phlm" | "Philemon" => "Phlm",
        "Heb" | "Hebrews" => "Heb",
        "Jas" | "James" => "Jas",
        "1Pet" | "1 Peter" => "1Pet",
        "2Pet" | "2 Peter" => "2Pet",
        "1John" | "1 John" => "1John",
        "2John" | "2 John" => "2John",
        "3John" | "3 John" => "3John",
        "Jude" => "Jude",
        "Rev" | "Revelation" => "Rev",
        _ if is_old_testament(book_name) || is_new_testament(book_name) => book_name,
        _ => book_name,
    }
    .to_string()
}

fn extract_interlinear_data(document: &Html, chapter: u32, book_name: &str) -> (Vec<InterlinearVerse>, Vec<CrossRef>) {
    let mut verses: Vec<InterlinearVerse> = Vec::new();
    let mut crossrefs: Vec<CrossRef> = Vec::new();
    let language_class = if is_old_testament(book_name) {
      "hebrew"
      } else if is_new_testament(book_name) {
          "greek"
      } else {
          "hebrew" // default fallback
      };

    // Select all elements that could be headers or word tables
    let chap_selector = Selector::parse("div.chap").unwrap();

    if let Some(chap_div) = document.select(&chap_selector).next() {
        let selector = if is_old_testament(book_name) {
            "table.tablefloatheb"
        } else if is_new_testament(book_name) {
            "table.tablefloat"
        } else {
            "table.tablefloatheb"
        };
        let table_selector = Selector::parse(selector).unwrap();

        let (
            reftop_selector,
            strongs_selector,
            reference_selector,
            translit_selector,
            original_selector,
            eng_selector,
            strongsnt_selector,
        ) = if is_new_testament(book_name) {
            (
                Selector::parse("span.reftop3").unwrap(),                 // verse number
                Selector::parse("span.pos a").unwrap(),                   // Strong’s ID
                Selector::parse("span.strongsnt2 a[href*='strongs']").unwrap(), // [e] link
                Selector::parse("span.translit a").unwrap(),              // transliteration
                Selector::parse("span.greek").unwrap(),                   // Greek
                Selector::parse("span.eng").unwrap(),                     // English gloss
                Selector::parse("span.strongsnt2 a").unwrap(),            // grammar tag
            )
        } else {
            (
                Selector::parse("span.reftop").unwrap(),
                Selector::parse("span.strongs a").unwrap(),
                Selector::parse("span.strongs a[href*='strongs']").unwrap(),
                Selector::parse("span.translit a").unwrap(),
                Selector::parse(&format!("span.{}", language_class)).unwrap(),
                Selector::parse("span.eng").unwrap(),
                Selector::parse("span.strongsnt a").unwrap(),
            )
        };


        let mut current_verse_num: Option<u32> = None;
        let mut current_words: Vec<InterlinearWord> = Vec::new();
        let mut verse_headers: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
        let mut pending_header: Option<String> = None;
        let mut pending_cross_links: Option<Vec<CrossRefLink>> = None;
        let cross_selector = Selector::parse("div.cross a").unwrap();

        for child in chap_div.children() {
            if let Some(elem_ref) = scraper::ElementRef::wrap(child) {
                match elem_ref.value().name() {
                    "div" => {
                        if elem_ref.value().attr("class") == Some("hdg") {
                            let mut header_text = String::new();
                            for node in elem_ref.children() {
                                if let Some(text) = node.value().as_text() {
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty() {
                                        header_text = trimmed.to_string();
                                        break;
                                    }
                                }
                            }

                            pending_header = if header_text.is_empty() {
                                None
                            } else {
                                Some(header_text)
                            };

                            let cross_links: Vec<CrossRefLink> = elem_ref
                                .select(&cross_selector)
                                .filter_map(|link| {
                                    let text = link.text().collect::<String>();
                                    parse_crossref(&text)
                                })
                                .collect();

                            pending_cross_links = if cross_links.is_empty() {
                                None
                            } else {
                                Some(cross_links)
                            };
                        }
                    }
                    "table" => {
                        if let Some(reftop) = elem_ref.select(&reftop_selector).next() {
                            let verse_text = reftop.text().collect::<String>().trim().to_string();
                            if let Ok(verse_num) = verse_text.parse::<u32>() {
                                if let Some(header) = pending_header.take() {
                                    verse_headers.insert(verse_num, header);
                                }

                                if let Some(mut links) = pending_cross_links.take() {
                                    if !links.is_empty() {
                                        if let Some(existing) = crossrefs
                                            .iter_mut()
                                            .find(|cr| cr.book == book_name && cr.chapter == chapter && cr.verse == verse_num)
                                        {
                                            existing.links.append(&mut links);
                                        } else {
                                            crossrefs.push(CrossRef {
                                                book: book_name.to_string(),
                                                chapter,
                                                verse: verse_num,
                                                links,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Second pass: extract word data
        use scraper::ElementRef;

        fn normalize_text(mut text: String) -> String {
            let nbsp = '\u{00A0}';
            if text.contains(nbsp) {
                text = text.replace(nbsp, " ");
            }
            text.trim().to_string()
        }

        fn first_nonempty_by_selector(root: &ElementRef, selector: &Selector, allow_dot: bool) -> String {
            root.select(selector)
                .map(|el| normalize_text(el.text().collect::<String>()))
                .find(|text| {
                    if text.is_empty() {
                        return false;
                    }
                    allow_dot || text != "."
                })
                .unwrap_or_default()
        }

        for table in document.select(&table_selector) {
            if let Some(reftop) = table.select(&reftop_selector).next() {
                let text = reftop.text().collect::<String>().trim().to_string();
                if let Ok(verse_num) = text.parse::<u32>() {
                    if let Some(v) = current_verse_num {
                        if v != verse_num && !current_words.is_empty() {
                            verses.push(InterlinearVerse {
                                chapter,
                                verse: v,
                                header: verse_headers.get(&v).cloned().unwrap_or_default(),
                                words: current_words.clone(),
                            });
                            current_words.clear();
                        }
                    }
                    current_verse_num = Some(verse_num);
                }
            }

            let strongs = table.select(&strongs_selector).next()
                .map(|s| s.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let reference = table.select(&reference_selector).next()
                .map(|s| s.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let transliteration = table.select(&translit_selector).next()
                .map(|s| normalize_text(s.text().collect::<String>()))
                .unwrap_or_default();

            let original = first_nonempty_by_selector(&table, &original_selector, false);

            let english = first_nonempty_by_selector(&table, &eng_selector, true);

            let grammar = table.select(&strongsnt_selector).next()
                .map(|s| normalize_text(s.text().collect::<String>()))
                .unwrap_or_default();

            if !strongs.is_empty() || !transliteration.is_empty() || !original.is_empty() {
                current_words.push(InterlinearWord {
                    strongs,
                    reference,
                    transliteration,
                    original,
                    english,
                    grammar,
                });
            }
        }

        // Save the last verse
        if let Some(v) = current_verse_num {
            if !current_words.is_empty() {
                verses.push(InterlinearVerse {
                    chapter,
                    verse: v,
                    header: verse_headers.get(&v).cloned().unwrap_or_default(),
                    words: current_words,
                });
            }
        }
    }

    (verses, crossrefs)
}

fn process_chapter_file(file_path: &Path, book_name: &str) -> Result<InterlinearChapterData, Box<dyn std::error::Error>> {
    let html_content = fs::read_to_string(file_path)?;
    let document = Html::parse_document(&html_content);

    let book_name_extracted = extract_book_name(&document).unwrap_or_else(|| book_name.to_string());
    let book_code = normalize_book_code(&book_name_extracted);
    let chapter = extract_chapter_number(&document).unwrap_or(1);

    let (verses, crossrefs) = extract_interlinear_data(&document, chapter, &book_name_extracted);

    Ok(InterlinearChapterData {
        translation_code: "interlinear".to_string(),
        translation_name: "Interlinear".to_string(),
        book_code,
        book_name: book_name_extracted,
        chapter,
        verses,
        crossrefs,
    })
}


pub(super) fn convert(path: &Path, book: &str, pretty: bool) -> anyhow::Result<Vec<super::Chapter>> {
    let data = process_chapter_file(path, book).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(vec![super::render(data.chapter, &data, pretty)?])
}
