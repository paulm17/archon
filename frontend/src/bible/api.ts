import { scriptureClient } from '../lib/client';
import type {
    BooksGroup,
    CategoryGroup,
    BookWithHeadings,
    Translation,
    PassageResult,
    GospelSelection,
} from './types';

// Use shared client from lib
export { scriptureClient as client };

function safeParseJson(input: string): any {
    try {
        return JSON.parse(input);
    } catch {
        return {};
    }
}

const translationsCache = new Map<GospelSelection, Translation[]>();
const THOMAS_EXCLUDED_TRANSLATION_CODES = new Set(['greek', 'coptic']);

export async function getBooksGrouped(translationCode: string): Promise<BooksGroup | null> {
    try {
        const query = `
            query GetBooks($code: String!) {
                queryBook {
                    id
                    code
                    nameEn
                    bookCategories {
                        id
                        category {
                            id
                            name
                        }
                    }
                    bookTranslations(filter: { translation: { code: { eq: $code } } }) {
                        id
                        chapters {
                            id
                            number
                            verses(filter: { number: { eq: 1 } }) {
                                id
                                number
                                chapterHeading
                                verseContents {
                                    id
                                    text
                                    type
                                }
                            }
                        }
                    }
                }
            }
        `;

        const result = await scriptureClient.query(query, { code: translationCode }).toPromise();

        if (result.error) {
            console.error("GraphQL Error:", result.error);
            return null;
        }

        if (!result.data?.queryBook) return null;

        // Group by category
        const categoriesMap = new Map<string, BookWithHeadings[]>();

        result.data.queryBook.forEach((book: any) => {
            const categoryName = book.bookCategories?.[0]?.category?.name || 'Other';
            
            // Extract chapters from bookTranslations
            const translation = book.bookTranslations?.[0]; // Should only be one due to filter
            let chapters = [];
            
            if (translation?.chapters) {
                chapters = translation.chapters.map((ch: any) => {
                    const verse1 = ch.verses?.[0];
                    // Prefer explicit heading, fallback to verse text
                    let heading = verse1?.chapterHeading || verse1?.verseContents?.find((vc: any) => vc.type === 'text')?.text || '';
                    
                    // Clean up metadata tags from heading (e.g. [vs:1], [cr:Gen 1:1], [fn:1])
                    if (heading) {
                        heading = heading.replace(/\[(?:vs|cr|fn|inline):[^\]]*\]\s*/g, '').trim();
                        // Also remove any leading verse references like "1:1" if they appear (though [vs:1] covers most)
                    }

                    return {
                        number: ch.number,
                        heading
                    };
                }).sort((a: any, b: any) => a.number - b.number);
            } else {
                 // Fallback if no translation data found for this book (e.g. Apocrypha in Protestant Bible)
                 // But wait, if book exists in queryBook but not in this translation, we might want to hide it?
                 // For now, let's just show it with 0 chapters or mock to verify UI structure if needed.
                 // Actually, if no chapters, user can't read it.
                 // Let's stick to empty chapters if none found.
            }

            const bookWithHeadings: BookWithHeadings = {
                id: book.id,
                code: book.code,
                name: book.nameEn,
                chapters
            };

            if (!categoriesMap.has(categoryName)) {
                categoriesMap.set(categoryName, []);
            }
            categoriesMap.get(categoryName)?.push(bookWithHeadings);
        });

        // Convert map to array and sort categories
        const categories: CategoryGroup[] = [];
        const order = ['Old Testament', 'New Testament', 'Apocrypha'];
        
        order.forEach(name => {
            if (categoriesMap.has(name)) {
                categories.push({
                    name,
                    books: categoriesMap.get(name)!
                });
            }
        });

        // Add any others
        categoriesMap.forEach((books, name) => {
            if (!order.includes(name)) {
                 categories.push({ name, books });
            }
        });

        return {
            translation: translationCode,
            language: null, 
            categories
        };

    } catch (error) {
        console.error("Error fetching books:", error);
        return null;
    }
}

export async function getTranslations(gospel: GospelSelection = 'canonical'): Promise<Translation[]> {
  const cached = translationsCache.get(gospel);
  if (cached) {
    return cached;
  }

  if (gospel === 'gospel-of-thomas') {
    try {
      const query = `
        query GetThomasTranslations {
          queryBook(filter: { bookCorpus: { eq: "GNOSTIC" }, code: { eq: "Thomas" } }) {
            code
            nameEn
            bookTranslations {
              translation {
                code
                name
              }
            }
          }
        }
      `;

      const result = await scriptureClient.query(query, {}).toPromise();
      if (result.error) {
        console.error('Thomas translations error:', result.error);
      }

      const books = result.data?.queryBook || [];
      const thomasBook = books[0];
      const byCode = new Map<string, Translation>();

      (thomasBook?.bookTranslations || []).forEach((bt: any) => {
        const code = bt?.translation?.code;
        const name = bt?.translation?.name;
        if (typeof code !== 'string' || !code || typeof name !== 'string' || !name) {
          return;
        }
        if (THOMAS_EXCLUDED_TRANSLATION_CODES.has(code.toLowerCase())) {
          return;
        }
        byCode.set(code, { code, name });
      });

      const translations = Array.from(byCode.values()).sort((a, b) => a.name.localeCompare(b.name));

      translationsCache.set(gospel, translations);
      return translations;
    } catch (error) {
      console.error('Error fetching Thomas translations:', error);
      return [];
    }
  }

  try {
    const query = `
      query GetCanonicalTranslations {
        queryBook(filter: { bookCorpus: { eq: "CANONICAL" } }) {
          id
          bookTranslations {
            id
            translation {
              id
              code
              name
            }
          }
        }
      }
    `;

    const result = await scriptureClient.query(query, {}).toPromise();
    if (result.error) {
      console.error('Canonical translations error:', result.error);
    }

    const books = result.data?.queryBook || [];
    const byCode = new Map<string, Translation>();
    books.forEach((book: any) => {
      (book.bookTranslations || []).forEach((bt: any) => {
        const code = bt?.translation?.code;
        const name = bt?.translation?.name;
        if (typeof code === 'string' && code && typeof name === 'string' && name) {
          byCode.set(code, { code, name });
        }
      });
    });

    const translations = Array.from(byCode.values()).sort((a, b) => a.name.localeCompare(b.name));
    translationsCache.set(gospel, translations);
    return translations;
  } catch (error) {
    console.error("Error fetching canonical translations:", error);
    return [];
  }
}

export async function getPassage(
  search: string,
  version: string,
  _opts: { includeFootnotes?: boolean; includeCrossrefs?: boolean; gospel?: GospelSelection } = {}
): Promise<PassageResult[]> {
    const gospelSelection = _opts.gospel ?? 'canonical';

    if (gospelSelection === 'gospel-of-thomas') {
        const normalizedThomasSearch = search.trim();
        const isThomasWholeBookSearch = /^(?:gospel\s+of\s+)?thomas$/i.test(normalizedThomasSearch);
        const thomasMatch = normalizedThomasSearch.match(
            /^(?:(?:gospel\s+of\s+)?thomas\s+)?(\d+)(?:-(\d+))?$/i
        );

        if (!isThomasWholeBookSearch && !thomasMatch) {
            return [];
        }

        const thomasVerseStart = thomasMatch ? parseInt(thomasMatch[1], 10) : 0;
        const thomasVerseEnd = thomasMatch?.[2] ? parseInt(thomasMatch[2], 10) : 0;

        const hierarchyQuery = `
            query GetThomasHierarchy($translationCode: String!) {
                queryBook(filter: { code: { eq: "Thomas" }, bookCorpus: { eq: "GNOSTIC" } }) {
                    id
                    code
                    bookTranslations(filter: { translation: { code: { eq: $translationCode } } }) {
                        id
                        translation { id code }
                        chapters {
                            id
                            number
                        }
                    }
                }
            }
        `;

        try {
            const hierarchyResult = await scriptureClient.query(hierarchyQuery, { translationCode: version }).toPromise();

            if (hierarchyResult.error) {
                console.error('GraphQL Error (Thomas hierarchy):', hierarchyResult.error);
                return [];
            }

            const books = hierarchyResult.data?.queryBook || [];
            if (books.length === 0) return [];

            const book = books[0];
            const translation = book.bookTranslations?.find((bt: any) => bt.translation?.code === version);
            if (!translation) {
                return [];
            }

            const allChapters = [...(translation.chapters || [])].sort(
                (a: any, b: any) => (a.number ?? 0) - (b.number ?? 0)
            );
            if (allChapters.length === 0) {
                return [];
            }

            const targetChapters = allChapters;
            if (targetChapters.length === 0) {
                return [];
            }

            const queryParts = targetChapters.map(
                (c: any, index: number) => `c${index}: getChapter(uid: "${c.id}") {
                    id
                    verses {
                        id
                        number
                        logionNumber
                        chapterHeading
                        sectionHeading
                        paragraphStart
                        poetryIndent
                        metadata
                        verseContents {
                            id
                            text
                            type
                        }
                    }
                }`
            );
            const contentQuery = `
                query GetThomasChapterContent {
                    ${queryParts.join('\n')}
                }
            `;

            const contentResult = await scriptureClient.query(contentQuery, {}).toPromise();
            if (contentResult.error) {
                console.error("GraphQL Error (Thomas content):", contentResult.error);
                return [];
            }

            let verses: any[] = [];
            targetChapters.forEach((c: any, index: number) => {
                const fetchedChapter = contentResult.data?.[`c${index}`];
                if (!fetchedChapter) {
                    return;
                }

                const chapterVerses = (fetchedChapter.verses || []).map((v: any) => {
                    if (v.metadata && typeof v.metadata === 'string') {
                        v = { ...v, metadata: safeParseJson(v.metadata) };
                    }
                    // Thomas is verse-centric. Prefer explicit logion number, then verse number,
                    // and finally chapter number (for one-verse-per-chapter layouts).
                    const resolvedThomasVerse =
                        (typeof v.logionNumber === 'number'
                            ? v.logionNumber
                            : typeof v.number === 'number' && v.number !== 1
                              ? v.number
                              : c.number) || 0;
                    return {
                        ...v,
                        number: resolvedThomasVerse,
                    };
                });

                verses.push(...chapterVerses);
            });

            if (!isThomasWholeBookSearch && thomasVerseStart > 0) {
                verses = verses.filter((v: any) => {
                    const num = v.number;
                    if (thomasVerseEnd > 0) return num >= thomasVerseStart && num <= thomasVerseEnd;
                    return num === thomasVerseStart;
                });
            }

            const deduped = new Map<string, any>();
            verses.forEach((v: any) => {
                const text = v.verseContents?.find((vc: any) => vc.type === 'text')?.text || '';
                const key = `${v.number}|${text}`;
                if (!deduped.has(key)) {
                    deduped.set(key, v);
                }
            });
            verses = Array.from(deduped.values()).sort((a: any, b: any) => a.number - b.number);

            if (verses.length === 0) {
                return [];
            }

            const displayVerses = verses.map((v: any) => ({
                ...v,
                paragraphStart: true,
            }));
            const html = renderVersesToHtml(displayVerses, { chapterNumber: 1 });
            const footnotes = buildFootnotesFromMetaAndMarkers(verses, 1);
            const crossrefs = buildCrossrefsFromMetaAndMarkers(verses, 1);

            return [{
                reference: {
                    bookCode: 'Thomas',
                    chapter: 1,
                    verseStart: thomasVerseStart,
                    verseEnd: thomasVerseEnd,
                },
                html,
                referenceText: search,
                isWholeChapter: false,
                maxChapter: 1,
                footnotes,
                crossrefs,
            }];
        } catch (error) {
            console.error('Error executing Thomas query:', error);
            return [];
        }
    }


    const match = search.match(/^([1-3]?[A-Za-z]+)\s+(\d+)(?::(\d+)(?:-(\d+))?)?$/);
    
    if (!match) {
         return [];
    }

    const [_, bookSearchStr, chapterStr, verseStartStr, verseEndStr] = match;
    const chapter = parseInt(chapterStr);
    const verseStart = verseStartStr ? parseInt(verseStartStr) : 0;
    const verseEnd = verseEndStr ? parseInt(verseEndStr) : 0;

    // Resolve book code
    // We fetch all books to do client-side matching because the input might be "Genesis" but code is "GEN"
    const booksQuery = `
      query GetAllBooks {
        queryBook {
          id
          code
          nameEn
        }
      }
    `;

    let bookCode = bookSearchStr;

    try {
        const booksResult = await scriptureClient.query(booksQuery, {}).toPromise();
        if (!booksResult.error && booksResult.data?.queryBook) {
            const books = booksResult.data.queryBook;
            const normalizedInfo = bookSearchStr.toLowerCase().trim();
            
            // 1. Check code exact
            let found = books.find((b: any) => b.code.toLowerCase() === normalizedInfo);
            
            // 2. Check name exact
            if (!found) found = books.find((b: any) => b.nameEn.toLowerCase() === normalizedInfo);
            
            // 3. Check name starts with
            if (!found) found = books.find((b: any) => b.nameEn.toLowerCase().startsWith(normalizedInfo));
            
            // 4. Check name includes
            if (!found) found = books.find((b: any) => b.nameEn.toLowerCase().includes(normalizedInfo));

        if (found) {
                bookCode = found.code;
            } else {
                 // fallback to as-is
            }
        }
    } catch (e) {
        console.error("Error fetching books for resolution:", e);
    }

    // Step 1: Get Chapter ID
    const hierarchyQuery = `
        query GetBookHierarchy($code: String!) {
            queryBook(filter: { code: { eq: $code } }) {
                id
                bookTranslations {
                    id
                    translation { id code }
                    chapters {
                        id
                        number
                    }
                }
            }
        }
    `;

    try {
        const hierarchyResult = await scriptureClient.query(hierarchyQuery, { code: bookCode }).toPromise();
        
        if (hierarchyResult.error) {
            console.error("GraphQL Error (Hierarchy):", hierarchyResult.error);
            return [];
        }

        const books = hierarchyResult.data?.queryBook || [];
        if (books.length === 0) return [];

        const book = books[0];


        const translation = book.bookTranslations?.find((bt: any) => bt.translation?.code === version);
        if (!translation) {
            return [];
        }

        const targetChapter = translation.chapters?.find((c: any) => c.number === chapter);
        if (!targetChapter) {
            return [];
        }

        // Step 2: Get Verses by Chapter ID
        // Note: Using getChapter for direct ID lookup
        const contentQuery = `
            query GetChapterContent($id: ID!) {
                getChapter(uid: $id) {
                    id
                    verses {
                        id
                        number
                        chapterHeading
                        sectionHeading
                        paragraphStart
                        poetryIndent
                        metadata
                        verseContents {
                            id
                            text
                            type
                        }
                    }
                }
            }
        `;

        const contentResult = await scriptureClient.query(contentQuery, { id: targetChapter.id }).toPromise();

        if (contentResult.error) {
             console.error("GraphQL Error (Content):", contentResult.error);
             return [];
        }

        const fetchedChapter = contentResult.data?.getChapter;
        if (!fetchedChapter) return [];

        let verses = fetchedChapter.verses || [];

        // Filter verses in JS
        if (verseStart > 0) {
            verses = verses.filter((v: any) => {
                const num = v.number;
                if (verseEnd > 0) return num >= verseStart && num <= verseEnd;
                return num === verseStart;
            });
        }

        // Sort verses by number
        verses.sort((a: any, b: any) => a.number - b.number);

        if (verses.length === 0) return [];

        // Parse metadata JSON strings into objects
        verses = verses.map((v: any) => {
             if (v.metadata && typeof v.metadata === 'string') {
                 try {
                     return { ...v, metadata: JSON.parse(v.metadata) };
                 } catch (e) {
                     return { ...v, metadata: {} };
                 }
             }
             return v;
        });

        // Format HTML using the shared renderer
        const html = renderVersesToHtml(verses, { chapterNumber: chapter });

        const footnotes = buildFootnotesFromMetaAndMarkers(verses, chapter);
        const crossrefs = buildCrossrefsFromMetaAndMarkers(verses, chapter);

        return [{
            reference: {
                bookCode,
                chapter,
                verseStart,
                verseEnd
            },
            html,
            referenceText: search,
            isWholeChapter: !verseStart,
            maxChapter: 50, 
            footnotes,
            crossrefs
        }];

    } catch (error) {
        console.error("Error executing query:", error);
        return [];
    }
}

export async function getPassageForAllPanes(panes: any[]): Promise<any[]> {
    return Promise.all(panes.map(async (pane) => {
        const data = await getPassage(pane.searchQuery, pane.version, { gospel: pane.gospel });
        return {
            paneId: pane.paneId,
            tabId: pane.tabId,
            data
        };
    }));
}

export const getVerseInAllTranslations = async (
  bookCode: string,
  chapter: number,
  verse: number
) => {
    // Step 1: Get all Chapter IDs for this book/chapter across all translations
    const hierarchyQuery = `
        query GetBookHierarchy($code: String!) {
            queryBook(filter: { code: { eq: $code } }) {
                id
                bookTranslations {
                    id
                    translation { id code name }
                    chapters {
                        id
                        number
                    }
                }
            }
        }
    `;

  try {
      const hierarchyResult = await scriptureClient.query(hierarchyQuery, { code: bookCode }).toPromise();
        
      if (hierarchyResult.error) {
          console.error("GraphQL Error (Hierarchy):", hierarchyResult.error);
          return { data: { verseTranslations: [] } };
      }

      const books = hierarchyResult.data?.queryBook || [];
      if (books.length === 0) return { data: { verseTranslations: [] } };

      const book = books[0];
      const chapterIds: string[] = [];
      const translationMap = new Map<string, any>(); // Map chapterID -> Translation info

      book.bookTranslations?.forEach((bt: any) => {
          const matchingChapter = bt.chapters?.find((c: any) => c.number === chapter);
          if (matchingChapter) {
              chapterIds.push(matchingChapter.id);
              translationMap.set(matchingChapter.id, bt.translation);
          }
      });

      if (chapterIds.length === 0) return { data: { verseTranslations: [] } };

      // Step 2: Fetch verses for these chapters
      // Note: Using aliases to fetch multiple chapters by ID since queryChapter filter doesn't support ID
      const queryParts = chapterIds.map((id, index) => `c${index}: getChapter(uid: "${id}") {
            id
            verses {
                id
                number
                verseContents {
                    id
                    text
                    type
                }
            }
      }`);
      
      const contentQuery = `
        query GetContent {
            ${queryParts.join('\n')}
        }
      `;

      const contentResult = await scriptureClient.query(contentQuery, {}).toPromise();
      
      if (contentResult.error) {
          console.error("GraphQL Error (Content):", contentResult.error);
          return { data: { verseTranslations: [] } }; // Return empty
      }

      // Collect results from aliases
      const returnedChapters: any[] = [];
      chapterIds.forEach((_, index) => {
          const ch = contentResult.data?.[`c${index}`];
          if (ch) returnedChapters.push(ch);
      });
      const verseTranslations: any[] = [];

      returnedChapters.forEach((ch: any) => {
          const targetVerse = ch.verses?.find((v: any) => v.number === verse);
          if (targetVerse) {
              const trans = translationMap.get(ch.id);
              const content = targetVerse.verseContents?.find((vc: any) => vc.type === 'text' || !vc.type) || targetVerse.verseContents?.[0];
              
              if (content && content.text && trans) {
                  verseTranslations.push({
                      text: content.text,
                      translation: trans
                  });
              }
          }
      });

      return { data: { verseTranslations } };

  } catch (err) {
      console.error(err);
      return { data: { verseTranslations: [] } };
  }
};

// --- Formatting Helpers reused from app/actions/getPassage.ts ---

function renderVersesToHtml(
  verses: any[],
  opts?: { includeFootnotes?: boolean; chapterNumber?: number }
): string {
  // De-duplicate verses by verse number to avoid duplicates from re-ingestion
  const unique = mergeDuplicateVerseRows(verses);

  const parts: string[] = [];
  let paragraph: string[] | null = null;
  let lastChapterHeading: string | null = null;
  let lastSectionHeading: string | null = null;

  const flushParagraph = () => {
    if (paragraph && paragraph.length) {
      parts.push(`<p>${paragraph.join(' ')}</p>`);
    }
    paragraph = null;
  };

  for (const v of unique) {
    const sectionHeading = deriveSectionHeading(v);
    const chapterHeading = deriveChapterHeading(v);
    
    // Inject marker processing here
    let content = ensureVerseMarker(withFootnoteAnchors(getVerseText(v), v.metadata, "verse"), v.number);
    content = processContentMarkers(content, v);

    const inlineHeading = deriveInlineHeading(v);

    // Emit section heading
    if (sectionHeading && sectionHeading !== lastSectionHeading) {
      flushParagraph();
      parts.push(`<h3>${sectionHeading}</h3>`);
      lastSectionHeading = sectionHeading;
    }

    // Emit chapter heading
    if (chapterHeading && chapterHeading !== lastChapterHeading) {
      flushParagraph();
      parts.push(`<h3>${withFootnoteAnchors(chapterHeading, v.metadata, "heading")}</h3>`);
      lastChapterHeading = chapterHeading;
    }

    // Poetry and Inline Markers
    // Remove a leading chapter prefix like "1 " at the start of chapter content (present on verse 1).
    let raw = stripLeadingChapterPrefix(content, opts?.chapterNumber ?? v.number, v.number);
    
    if (inlineHeading) {
        const inlineMarker = `[inline:${inlineHeading.replace(/\]/g, '&#93;')}]`;
        if (raw && raw.includes('[inline]')) {
            raw = raw.replace(/\[inline\]/g, `${inlineMarker} `);
        } else if (!/\[inline:/.test(raw ?? '')) {
            raw = `${inlineMarker} ${raw}`.trim();
        }
    } else if (raw && raw.includes('[inline]')) {
        raw = raw.replace(/\[inline\]/g, '');
    }

    // Normalize literal "\n" sequences into real newlines
    let normalized = (raw ?? '').replace(/\\n/g, '\n');
    normalized = normalized.replace(/([,;.!?"”’\)\]])\s*:\s*/g, '$1\n: ');
    normalized = normalized.replace(/([A-Za-z”’"\)\]])\s*:\s+(?=[a-z])/g, '$1\n: ');
    
    const hadPoetryBreaks = /(?:^|\n):\s/.test(raw ?? '');
    if (!hadPoetryBreaks) {
      normalized = normalized.replace(/\n:\s+/g, ': ');
    }
    const lines = normalized.split('\n');

    for (const line of lines) {
      const m = line.match(/^(:+)\s*(.*)$/);
      if (m) {
        // Poetry line
        flushParagraph();
        const indent = m[1].length; 
        const text = m[2];
        const style = indent > 0 ? ` style="margin-left: ${indent * 2}em"` : '';
        parts.push(`<div class="poetry-line indent-${indent}"${style}><span>${text}</span></div>`);
      } else {
        // Prose
        if (v.paragraphStart || !paragraph) {
          flushParagraph();
          paragraph = [];
        }
        if (!paragraph) paragraph = [];
        paragraph.push(`<span>${line}</span>`);
      }
    }
  }

  flushParagraph();
  return parts.join('\n');
}

function mergeDuplicateVerseRows(verses: any[]): any[] {
  const map = new Map<number, any>();
  for (const verse of verses) {
    const key = verse.number;
    const existing = map.get(key);
    if (!existing) {
      map.set(key, { ...verse, verseContents: [...(verse.verseContents ?? [])] });
      continue;
    }
    if (!existing.chapterHeading && verse.chapterHeading) existing.chapterHeading = verse.chapterHeading;
    if (!existing.sectionHeading && verse.sectionHeading) existing.sectionHeading = verse.sectionHeading;
    if (verse.paragraphStart) existing.paragraphStart = true;
    if ((verse.poetryIndent ?? 0) > (existing.poetryIndent ?? 0)) existing.poetryIndent = verse.poetryIndent;
    
    if (Array.isArray(verse.verseContents) && verse.verseContents.length > 0) {
      existing.verseContents.push(...verse.verseContents);
    }
  }
  return Array.from(map.values()).sort((a, b) => a.number - b.number);
}

function deriveSectionHeading(v: any): string | null {
  return v.sectionHeading || null;
}

function deriveChapterHeading(v: any): string | null {
  return v.chapterHeading || null;
}

function deriveInlineHeading(v: any): string | null {
   if (v.metadata) {
       try {
           const meta = typeof v.metadata === 'string' ? JSON.parse(v.metadata) : v.metadata;
           return meta?.inlineHeading || meta?.inline_heading || meta?.title || null;
       } catch (e) { return null; }
   }
   return null;
}

type AnchoredFootnote = { id: string; offset: number; target: string };

function withFootnoteAnchors(text: string, metadata: { footnotes?: AnchoredFootnote[] } | null, target: string) {
  const notes = Array.isArray(metadata?.footnotes) ? metadata.footnotes : [];
  const anchors = notes.filter(note => note.target === target && Number.isInteger(note.offset)
    && note.offset >= 0 && note.offset <= text.length);
  // Insert right to left so offsets still refer to the original verse.
  for (const note of anchors.reverse().sort((a, b) => b.offset - a.offset)) {
    text = text.slice(0, note.offset) + `[note:${encodeURIComponent(note.id)}]` + text.slice(note.offset);
  }
  return text;
}

function getVerseText(v: any): string {
  const contents = Array.isArray(v?.verseContents) ? v.verseContents : [];
  return contents
    .map((entry: any) => String(entry?.text ?? '').trim())
    .filter((t: string) => t.length > 0)
    .join(' ')
    .trim();
}

function ensureVerseMarker(text: string, verseNumber: number): string {
  const content = text ?? '';
  if (/\[vs:\d+\]/.test(content)) return content;
  const prefix = `[vs:${verseNumber}]`;
  return content ? `${prefix} ${content}` : prefix;
}

function stripLeadingChapterPrefix(text: string, chapter: number, verseNumber: number): string {
  const trimmed = text?.trimStart() ?? '';
  if (verseNumber === 1) {
    const specific = String(chapter) + ' ';
    if (trimmed.startsWith(specific)) return trimmed.slice(specific.length);
    return trimmed.replace(/^\s*\d+\s+/, '');
  }
  return trimmed;
}

// Helpers to extract footnotes and crossrefs from Verse.metadata plus inline markers in content
function buildFootnotesFromMetaAndMarkers(verses: any[], chapterNumber: number): any[] {
  const items: any[] = [];
  for (const v of verses) {
    const content = getVerseText(v);
    const meta = v.metadata ?? {};
    const markerSet = new Set<string>();

    // Collect markers from inline content
    for (const m of content.matchAll(/\[fn:([A-Za-z]+)\]/g)) {
      markerSet.add(m[1]);
    }

    // Also collect markers from meta (e.g., footnotes array) for this verse
    const metaArr = Array.isArray(meta.footnotes)
      ? meta.footnotes
      : Array.isArray(meta.fn)
        ? meta.fn
        : null;

    if (metaArr) {
      for (const entry of metaArr) {
        if (!entry) continue;
        if (Number.isInteger(entry.offset) && entry.id) {
          items.push({ ...entry, chapter: chapterNumber, verse: v.number, noteId: entry.id });
          continue;
        }
        const marker =
          (entry.marker as string) || (entry.id as string) || (entry.noteId as string) || '';
        if (marker) {
          markerSet.add(marker);
        }
      }
    }

    // If no markers found but meta has footnotes, try to extract keys from meta.fn or meta.footnotes objects
    if (markerSet.size === 0) {
      if (meta.fn && typeof meta.fn === 'object' && !Array.isArray(meta.fn)) {
        Object.keys(meta.fn).forEach((key) => markerSet.add(key));
      }
      if (meta.footnotes && typeof meta.footnotes === 'object' && !Array.isArray(meta.footnotes)) {
        Object.keys(meta.footnotes).forEach((key) => markerSet.add(key));
      }
    }

    for (const id of markerSet) {
      const source = resolveFootnoteMeta(meta, id);
        // Ensure unique noteId by incorporating chapter and verse
      items.push({
        chapter: chapterNumber,
        verse: v.number,
        noteId: id.length === 1 ? `${chapterNumber}:${v.number}-${id}` : id,
        text: source?.text ?? '',
        references: source?.references,
      });
    }
  }

  // Deduplicate by noteId
  const uniqueItems = Array.from(new Map(items.map((item) => [item.noteId, item])).values());

  return uniqueItems.sort((a, b) =>
    a.chapter !== b.chapter
      ? a.chapter - b.chapter
      : a.verse !== b.verse
        ? a.verse - b.verse
        : a.noteId.localeCompare(b.noteId)
  );
}

function resolveFootnoteMeta(meta: any, id: string): any {
    if (!meta) return null;
    // 1. Check array: meta.footnotes or meta.fn
    const arr = Array.isArray(meta.footnotes) ? meta.footnotes : Array.isArray(meta.fn) ? meta.fn : null;
    if (arr) {
        return arr.find((f: any) => f.id === id || f.marker === id || f.noteId === id) || null;
    }
    // 2. Check object keys
    if (meta.footnotes && !Array.isArray(meta.footnotes) && meta.footnotes[id]) return meta.footnotes[id];
    if (meta.fn && !Array.isArray(meta.fn) && meta.fn[id]) return meta.fn[id];
    return null;

}

function buildCrossrefsFromMetaAndMarkers(verses: any[], chapterNumber: number): any[] {
  const items: any[] = [];
  for (const v of verses) {
    const content = getVerseText(v);
    const meta = v.metadata ?? {};
    const markerSet = new Set<string>();
    // Collect markers from inline content
    for (const m of content.matchAll(/\[cr:([A-Za-z]+)\]/g)) {
      markerSet.add(m[1]);
    }
    // Also collect markers from meta (e.g., cross_refs array) for this verse
    const metaArr = Array.isArray(meta.cross_refs)
      ? meta.cross_refs
      : Array.isArray(meta.crossrefs)
        ? meta.crossrefs
        : Array.isArray(meta.cr)
          ? meta.cr
          : Array.isArray(meta.cross_references)
            ? meta.cross_references
            : null;
    if (metaArr) {
      for (const entry of metaArr) {
        if (!entry) continue;
        const parsed = parseCrossRefMetaId(entry.id ?? entry.refId ?? '');
        const marker = (entry.marker as string) || parsed?.label || '';
        if (
          marker &&
          parsed &&
          parsed.chapter === chapterNumber &&
          (parsed.verse || 0) === v.number
        ) {
          markerSet.add(marker);
        }
      }
    }

    for (const id of markerSet) {
      const source = findCrossrefMetaForMarker(meta, id, chapterNumber, v.number);
      items.push({
        chapter: chapterNumber,
        verse: v.number,
        // Always include chapter:verse in the refId so the UI can format base refs consistently
        refId: `${chapterNumber}:${v.number}-${id}`,
        // Build links from known shapes. Prefer array of objects; otherwise parse string refs like "Psalms.102.25"
        links: buildCrossrefLinksFromSource(source),
      });
    }
  }

  // Deduplicate by refId
  const uniqueItems = Array.from(new Map(items.map((item) => [item.refId, item])).values());
  return uniqueItems;
}

function parseCrossRefMetaId(id: string): { chapter: number; verse: number; label: string } | null {
    // Example format: "GEN.1.1-A", "1.1-a", etc.
    // This regex is approximate based on common dataset patterns
    const match = id.match(/(?:[A-Za-z0-9]+\.)?(\d+)\.(\d+)-([A-Za-z]+)/);
    if (!match) return null;
    return {
        chapter: parseInt(match[1]),
        verse: parseInt(match[2]),
        label: match[3]
    };
}

function findCrossrefMetaForMarker(meta: any, marker: string, chapter: number, verse: number): any {
    if (!meta) return null;
    const arr = Array.isArray(meta.cross_refs) ? meta.cross_refs : Array.isArray(meta.crossrefs) ? meta.crossrefs : Array.isArray(meta.cr) ? meta.cr : null;

    if (arr) {
        // Try to find by marker property
        let found = arr.find((x: any) => x.marker === marker);
        if (found) return found;

        // Try to find by ID if ID matches pattern consistent with chapter/verse/marker
        // This is a fuzzy match because ID formats vary
        found = arr.find((x: any) => {
             const parsed = parseCrossRefMetaId(x.id || x.refId || '');
             return parsed && parsed.chapter === chapter && parsed.verse === verse && parsed.label === marker;
        });
        if (found) return found;
    }
    return null;
}

function buildCrossrefLinksFromSource(source: any): any[] {
    if (!source) return [];
    
    let refs: any[] = [];
    if (Array.isArray(source.refs)) refs = source.refs;
    else if (Array.isArray(source.references)) refs = source.references;
    else return [];

    return refs.map(ref => {
        if (typeof ref === 'string') {
            // Parse "Book.Chapter.Verse" or "Book.Chapter"
            // Example: "Psalms.102.25" -> { book: "Psalms", chapter: 102, verse: 25 }
            const parts = ref.split('.');
            if (parts.length >= 2) {
                const book = parts[0];
                const chapter = parseInt(parts[1]);
                const verse = parts.length > 2 ? parseInt(parts[2]) : undefined;
                return { book, chapter, verse };
            }
            return { book: ref };
        }
        return ref;
    });
}

function processContentMarkers(text: string, v: any): string {
    if (!text) return '';
    
    // Transform [A], [B], etc. into [fn:a], [fn:b] or [cr:A], [cr:B]
    // Strategy: Look for short bracketed sequences. Check if they exist in footnotes or crossrefs metadata.
    // Footnotes are usually lowercase in VardaDB content? Or matching the [A] in text?
    // The user screenshot showed [A] in text, and [fn:a] in tooltip map logic in PassageSection.
    // PassageSection expects [fn:a] or [cr:A].
    
    return text.replace(/\[([A-Za-z]+)\]/g, (match, label) => {
        // preserve existing typed markers
        if (text.includes(`[fn:${label}]`) || text.includes(`[cr:${label}]`) || text.includes(`[vs:${label}]`) || text.includes(`[inline:${label}]`)) {
             return match; 
        }

        // naive heuristic: 
        // 1. Check if 'label' is a footnote ID in metadata
        // 2. Check if 'label' is a crossref ID in markers
        
        const meta = v.metadata || {};
        
        // Check footnotes (case-insensitive match often needed)
        const fnArr = Array.isArray(meta.footnotes) ? meta.footnotes : Array.isArray(meta.fn) ? meta.fn : null;
        if (fnArr) {
            const found = fnArr.find((f: any) => 
                (f.id === label || f.marker === label || f.noteId === label) ||
                (f.id === label.toLowerCase() || f.marker === label.toLowerCase())
            );
            if (found) return `[fn:${found.id || found.marker || label}]`;
        }
        
        // Check crossrefs
        const crArr = Array.isArray(meta.cross_refs) ? meta.cross_refs : Array.isArray(meta.crossrefs) ? meta.crossrefs : Array.isArray(meta.cr) ? meta.cr : null;
        if (crArr) {
             const found = crArr.find((c: any) => 
                (c.marker === label) || 
                (c.id && c.id.endsWith(`-${label}`))
             );
             if (found) return `[cr:${label}]`;
        }

        // Fallback: If it looks like a single letter, default to footnote for lowercase, crossref for uppercase?
        // User screenshot: "In the beginning[A]" -> Footnote A.
        // "God created[B]" -> Footnote B.
        // Wait, footnotes list shows A, B, C... 
        // The PassageSection logic seems to use `[fn:a]` (lowercase) for rendering?
        // Let's check PassageSection.tsx lines 53-54: 
        // "if (!marker && /^[A-Za-z]$/.test(String(fn.noteId ?? ''))) marker = String(fn.noteId).toLowerCase();"
        // So the component renders `[a]`.
        // If we convert `[A]` to `[fn:A]`, PassageSection logic:
        // `else if (kind === 'fn') { ... const label = String(val).toLowerCase(); ...`
        // So `[fn:A]` becomes label `a`.
        
        // So, is it a footnote or crossref?
        // In the user's debug output for crossrefs: refId "1:1-A", "1:1-B".
        // In the user's debug output for footnotes: noteId "1:26-a".
        // It seems crossrefs use uppercase A, B. Footnotes use lowercase a?
        // BUT the screenshot shows `[A]` in text for verses 1.
        // And the Bottom Panel shows "A. Genesis 1:1 ...". That looks like cross-references!
        // The Footnotes tab shows "a. Genesis 1:26 ...".
        
        // So `[A]` in text is likely a Cross Reference. `[a]` would be a footnote.
        // Let's try to infer type based on presence in metadata.
        
        if (crArr && crArr.some((c: any) => c.marker === label || (c.id && c.id.endsWith(`-${label}`)))) {
            return `[cr:${label}]`;
        }
        
        if (fnArr && fnArr.some((f: any) => f.marker === label || f.id === label)) {
             return `[fn:${label}]`;
        }
        
        // Default to cr if uppercase, fn if lowercase?
        if (/^[A-Z]+$/.test(label)) return `[cr:${label}]`;
        if (/^[a-z]+$/.test(label)) return `[fn:${label}]`;

        return match;
    });
}
