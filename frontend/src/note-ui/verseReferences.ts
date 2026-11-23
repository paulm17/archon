import type { NoteVerseInput } from './api';

const BOOK_ALIASES: Array<[string, string]> = [
  ['gen', 'Genesis'], ['genesis', 'Genesis'],
  ['exod', 'Exodus'], ['exo', 'Exodus'], ['exodus', 'Exodus'],
  ['lev', 'Leviticus'], ['leviticus', 'Leviticus'],
  ['num', 'Numbers'], ['numbers', 'Numbers'],
  ['deut', 'Deuteronomy'], ['deuteronomy', 'Deuteronomy'],
  ['josh', 'Joshua'], ['joshua', 'Joshua'],
  ['judg', 'Judges'], ['judges', 'Judges'],
  ['ruth', 'Ruth'],
  ['1 sam', '1 Samuel'], ['1 samuel', '1 Samuel'],
  ['2 sam', '2 Samuel'], ['2 samuel', '2 Samuel'],
  ['1 kgs', '1 Kings'], ['1 kings', '1 Kings'],
  ['2 kgs', '2 Kings'], ['2 kings', '2 Kings'],
  ['1 chr', '1 Chronicles'], ['1 chronicles', '1 Chronicles'],
  ['2 chr', '2 Chronicles'], ['2 chronicles', '2 Chronicles'],
  ['ezra', 'Ezra'], ['neh', 'Nehemiah'], ['nehemiah', 'Nehemiah'],
  ['esth', 'Esther'], ['esther', 'Esther'],
  ['job', 'Job'], ['ps', 'Psalms'], ['psalm', 'Psalms'], ['psalms', 'Psalms'],
  ['prov', 'Proverbs'], ['proverbs', 'Proverbs'],
  ['eccl', 'Ecclesiastes'], ['ecclesiastes', 'Ecclesiastes'],
  ['song', 'Song of Solomon'], ['song of songs', 'Song of Solomon'], ['song of solomon', 'Song of Solomon'],
  ['isa', 'Isaiah'], ['isaiah', 'Isaiah'],
  ['jer', 'Jeremiah'], ['jeremiah', 'Jeremiah'],
  ['lam', 'Lamentations'], ['lamentations', 'Lamentations'],
  ['ezek', 'Ezekiel'], ['ezekiel', 'Ezekiel'],
  ['dan', 'Daniel'], ['daniel', 'Daniel'],
  ['hos', 'Hosea'], ['hosea', 'Hosea'],
  ['joel', 'Joel'], ['amos', 'Amos'], ['obad', 'Obadiah'], ['obadiah', 'Obadiah'],
  ['jonah', 'Jonah'], ['mic', 'Micah'], ['micah', 'Micah'],
  ['nah', 'Nahum'], ['nahum', 'Nahum'],
  ['hab', 'Habakkuk'], ['habakkuk', 'Habakkuk'],
  ['zeph', 'Zephaniah'], ['zephaniah', 'Zephaniah'],
  ['hag', 'Haggai'], ['haggai', 'Haggai'],
  ['zech', 'Zechariah'], ['zechariah', 'Zechariah'],
  ['mal', 'Malachi'], ['malachi', 'Malachi'],
  ['matt', 'Matthew'], ['matthew', 'Matthew'],
  ['mark', 'Mark'], ['luke', 'Luke'], ['john', 'John'],
  ['acts', 'Acts'], ['rom', 'Romans'], ['romans', 'Romans'],
  ['1 cor', '1 Corinthians'], ['1 corinthians', '1 Corinthians'],
  ['2 cor', '2 Corinthians'], ['2 corinthians', '2 Corinthians'],
  ['gal', 'Galatians'], ['galatians', 'Galatians'],
  ['eph', 'Ephesians'], ['ephesians', 'Ephesians'],
  ['phil', 'Philippians'], ['philippians', 'Philippians'],
  ['col', 'Colossians'], ['colossians', 'Colossians'],
  ['1 thess', '1 Thessalonians'], ['1 thessalonians', '1 Thessalonians'],
  ['2 thess', '2 Thessalonians'], ['2 thessalonians', '2 Thessalonians'],
  ['1 tim', '1 Timothy'], ['1 timothy', '1 Timothy'],
  ['2 tim', '2 Timothy'], ['2 timothy', '2 Timothy'],
  ['titus', 'Titus'], ['phlm', 'Philemon'], ['philemon', 'Philemon'],
  ['heb', 'Hebrews'], ['hebrews', 'Hebrews'],
  ['jas', 'James'], ['james', 'James'],
  ['1 pet', '1 Peter'], ['1 peter', '1 Peter'],
  ['2 pet', '2 Peter'], ['2 peter', '2 Peter'],
  ['1 john', '1 John'], ['2 john', '2 John'], ['3 john', '3 John'],
  ['jude', 'Jude'], ['rev', 'Revelation'], ['revelation', 'Revelation'],
];

const BOOK_NAME_PATTERN = '(?:[1-3]\\s+)?[A-Za-z]+(?:\\s+[A-Za-z]+)*';

export const VERSE_INPUT_PATTERN = new RegExp(
  `^\\s*(${BOOK_NAME_PATTERN})\\s+(\\d+):(\\d+)(?:\\s*[-–]\\s*(\\d+))?\\s*$`,
);

export const VERSE_GLOBAL_PATTERN = new RegExp(
  `(^|[^A-Za-z0-9])(${BOOK_NAME_PATTERN})\\s+(\\d+):(\\d+)(?:\\s*[-–]\\s*(\\d+))?\\b`,
  'g',
);

function normalizeBookName(input: string) {
  return input
    .trim()
    .trimEnd()
    .replace(/\.$/, '')
    .split(/\s+/)
    .join(' ')
    .toLowerCase();
}

export function canonicalizeBook(input: string): string | null {
  const normalized = normalizeBookName(input);
  const match = BOOK_ALIASES.find(([alias]) => alias === normalized);
  return match?.[1] ?? null;
}

export function isValidVerseReference(fullRef: string): boolean {
  const match = fullRef.match(VERSE_INPUT_PATTERN);
  if (!match) {
    return false;
  }
  return canonicalizeBook(match[1]) !== null;
}

export function extractVersesFromContent(input: string): NoteVerseInput[] {
  const seen = new Set<string>();
  const extracted: NoteVerseInput[] = [];

  for (const match of input.matchAll(VERSE_GLOBAL_PATTERN)) {
    const gospel = canonicalizeBook(match[2]);
    if (!gospel) {
      continue;
    }

    const chapter = match[3];
    const start = match[4];
    const end = match[5];
    const reference = end ? `${chapter}:${start}-${end}` : `${chapter}:${start}`;
    const fullRef = `${gospel} ${reference}`;
    const key = fullRef.toLowerCase();
    if (seen.has(key)) {
      continue;
    }

    seen.add(key);
    extracted.push({ id: fullRef, gospel, reference, fullRef });
  }

  return extracted;
}
