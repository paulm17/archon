import { DragDropProvider, DragOverlay } from '@dnd-kit/react';
import { isSortable, useSortable } from '@dnd-kit/react/sortable';
import { useEffect, useRef, useState } from 'react';
import {
  Alert,
  Badge,
  Button,
  Container,
  Drawer,
  Group,
  Loader,
  Stack,
  Table,
  Tabs,
  Text,
  TextInput,
  Textarea,
  Tooltip,
  Tree,
  Title,
  useTree,
  type TreeNodeData,
} from '@mantine/core';
import { IconGripVertical, IconHighlight, IconTrash } from '@tabler/icons-react';
import { useDebouncedState } from '@mantine/hooks';
import { executeVardaRequest } from '../lib/client';
import { useGospelStore } from '../lib/store';
import { ComparisonPage } from '../comparison/comparisonPage';
import styles from './newGospelPage.module.css';

type SupportingReferenceDisplay = {
  display: string;
};

type VerseMark = {
  id: string;
  rowKey: string;
  translation: string;
  reference: string;
  text: string;
  start: number;
  end: number;
};

type NewGospelVerseRecord = {
  uid: string;
  number: number;
  pericope: string | null;
  text: string;
  notes: string | null;
  assembledVerseMarks: string | null;
  supportingReferences: SupportingReferenceDisplay[];
};

type NewGospelVerseQueryResult = {
  queryNewGospelVerse?: Array<{
    uid?: string | null;
    number?: number | null;
    pericope?: string | null;
    text?: string | null;
    notes?: string | null;
    assembledVerseMarks?: string | null;
    supportingReferences?: Array<{
      display?: string | null;
    } | null> | null;
  }> | null;
};

type NewGospelVerseNumbersQueryResult = {
  queryNewGospelVerse?: Array<{
    number?: number | null;
    pericope?: string | null;
  }> | null;
};

type BookInfo = {
  code: string;
  name: string;
};

type BooksQueryResult = {
  queryBook?: Array<{
    id?: string | null;
    code?: string | null;
    nameEn?: string | null;
  }> | null;
};

type ParsedReference = {
  display: string;
  bookCode: string;
  chapter: number;
  startVerse: number;
  endVerse: number;
};

type ReferenceContext = {
  bookCode: string;
  chapter: number;
};

type ArchonVerseContent = {
  text: string;
  translationName: string;
  translationCode: string | null;
  lemma: string | null;
  strong: string | null;
  morph: string | null;
  gloss: string | null;
  tokenIndex: number | null;
};

type InterlinearToken = {
  tokenIndex: number;
  strong: string | null;
  reference: string | null;
  transliteration: string | null;
  original: string | null;
  englishGloss: string | null;
  grammar: string | null;
};

type ArchonVerseRecord = {
  number: number;
  verseContents: ArchonVerseContent[];
  interlinearTokens: InterlinearToken[];
};

type ArchonVerseQueryResult = {
  queryVerse?: Array<{
    number?: number | null;
    verseContents?: Array<{
      text?: string | null;
      type?: string | null;
      lemma?: string | null;
      strong?: string | null;
      morph?: string | null;
      gloss?: string | null;
      tokenIndex?: number | null;
      translation?: {
        code?: string | null;
        name?: string | null;
      } | null;
    } | null> | null;
    interlinearTokens?: Array<{
      tokenIndex?: number | null;
      strong?: string | null;
      reference?: string | null;
      transliteration?: string | null;
      original?: string | null;
      englishGloss?: string | null;
      grammar?: string | null;
    } | null> | null;
    alignments?: Array<{
      wordIndex?: number | null;
      hebrewWord?: string | null;
      englishGloss?: string | null;
      lemma?: {
        strongsId?: string | null;
        lemmaText?: string | null;
        gloss?: string | null;
      } | null;
    } | null> | null;
  }> | null;
};

type FixedInterlinearQueryResult = {
  queryVerse?: Array<{
    alignments?: Array<{
      wordIndex?: number | null;
      hebrewWord?: string | null;
      englishGloss?: string | null;
      lemma?: {
        strongsId?: string | null;
        lemmaText?: string | null;
        gloss?: string | null;
      } | null;
    } | null> | null;
    interlinearTokens?: Array<{
      tokenIndex?: number | null;
      strong?: string | null;
      reference?: string | null;
      transliteration?: string | null;
      original?: string | null;
      englishGloss?: string | null;
      grammar?: string | null;
    } | null> | null;
  } | null> | null;
};

type LinkedVerseRow = {
  key: string;
  translationName: string;
  display: string;
  referenceLabel: string;
  verseNumber: number;
  text: string;
};

type InterlinearRow = {
  key: string;
  translationName: string;
  display: string;
  referenceLabel: string;
  tokens: InterlinearToken[];
};

type InterlinearReference = {
  bookCode: string;
  chapter: number;
  verseNumber: number;
};

type PericopeVerseEntry = {
  number: number;
  pericope: string | null;
};

type ComparisonMetricRecord = {
  comparisonKind?: string | null;
  rightTranslationCode?: string | null;
  ngramContainmentPct?: number | null;
  sequenceSimilarityPct?: number | null;
  tokenOverlapPct?: number | null;
  semanticSimilarityPct?: number | null;
  longestCommonRunWords?: number | null;
  longestCommonRunText?: string | null;
  baselineDeltaPct?: number | null;
};

type ComparisonSnapshotInfo = {
  thresholdPct: number;
  verbatimRunFlagWords: number;
};

type ComparisonVerseRecord = {
  snapshot?: {
    generatedAt?: string | null;
    thresholdPct?: number | null;
    verbatimRunFlagWords?: number | null;
  } | null;
  metrics?: ComparisonMetricRecord[] | null;
};

type ComparisonVerseQueryResult = {
  queryScriptureComparisonVerse?: ComparisonVerseRecord[] | null;
};

const PRIORITY_TRANSLATIONS = [
  'Interlinear',
  'Open English Bible',
];
const BIBLE_TRANSLATIONS = new Map([
  ["OEB", "Open English Bible"],
  ["ASV", "American Standard Version"],
  ["AKJV", "Authorized (King James) Version"],
  ["BSB", "Berean Standard Bible"],
  ["DARBY", "Darby Translation"],
  ["DRA", "Douay-Rheims 1899 American Edition"],
  ["KJV", "King James Version"],
  ["TCNT", "Twentieth Century New Testament"],
  ["TWENTIETH_CENTURY", "Twentieth Century New Testament"], // Legacy import code
  ["WNT", "Weymouth New Testament"],
  ["WEB", "World English Bible"],
  ["YLT", "Young's Literal Translation"],
  ["T4T", "Translation for Translators"],
  ["LSV", "Literal Standard Version"],
  ["FBV", "Free Bible Version"],
  ["MSB", "Majority Standard Bible"],
  ["ULT", "unfoldingWord Literal Text"],
  ["UST", "unfoldingWord Simplified Text"],
  ["WMBB", "World Messianic Bible \u2014 British Edition"],
  ["NHEB", "New Heart English Bible"],
]);
const BIBLE_TRANSLATION_NAMES = new Set(BIBLE_TRANSLATIONS.values());
const EXCLUDED_TRANSLATIONS = ['NIV', 'ESV', 'NKJV', 'NLT', 'CSB', 'NRSVUE'];
const FIRST_COLUMN_WIDTH_PX = 400;

function isThomasBookCode(bookCode: string): boolean {
  const normalized = bookCode.trim().toLowerCase();
  return normalized === 'thomas' || normalized === 'gospelofthomas' || normalized === 'gospel_of_thomas';
}

function isNagHammadiCopticTranslation(translationName: string): boolean {
  return translationName.trim().toLowerCase() === 'nag hammadi coptic text';
}

function isThomasCopticMachineTranslation(translationName: string): boolean {
  return translationName.trim().toLowerCase() === 'coptic machine';
}

function scoreTone(score: number): string {
  if (score >= 40) return '#743636';
  if (score >= 20) return '#245f39';
  return '#363636';
}

function formatPct(value: number | null | undefined): string {
  if (typeof value !== 'number' || Number.isNaN(value)) {
    return '0%';
  }
  return `${Math.round(value)}%`;
}

function matchBook(abbrev: string, books: BookInfo[]): string | null {
  const normalized = abbrev.toLowerCase().trim();
  const normalizedCompact = normalized.replace(/\s+/g, '');

  if (
    normalized === 'gospel of thomas'
    || normalized === 'thomas'
    || normalized === 'thom'
    || normalizedCompact === 'gospelofthomas'
  ) {
    const thomasBook = books.find((book) => isThomasBookCode(book.code))
      ?? books.find((book) => book.name.toLowerCase().includes('thomas'));
    if (thomasBook) {
      return thomasBook.code;
    }
  }

  for (const book of books) {
    if (book.code.toLowerCase() === normalized) return book.code;
  }

  for (const book of books) {
    if (book.name.toLowerCase() === normalized) return book.code;
  }

  for (const book of books) {
    if (book.name.toLowerCase().startsWith(normalized)) return book.code;
  }

  for (const book of books) {
    if (book.name.toLowerCase().includes(normalized)) return book.code;
  }

  return null;
}

function parseSingleReferenceSegment(
  segment: string,
  books: BookInfo[],
  context: ReferenceContext | null,
): ParsedReference | null {
  const cleaned = segment.trim();
  const fullThomasReferenceMatch = cleaned.match(
    /^([1-3]?\s*[A-Za-z]+(?:\s+[A-Za-z]+)*)\s+(\d+)(?:-(\d+))?$/,
  );

  if (fullThomasReferenceMatch) {
    const bookCode = matchBook(fullThomasReferenceMatch[1], books);
    if (bookCode && isThomasBookCode(bookCode)) {
      const startVerse = Number.parseInt(fullThomasReferenceMatch[2], 10);
      const endVerse = fullThomasReferenceMatch[3]
        ? Number.parseInt(fullThomasReferenceMatch[3], 10)
        : startVerse;

      if (
        Number.isNaN(startVerse)
        || Number.isNaN(endVerse)
        || endVerse < startVerse
      ) {
        return null;
      }

      return {
        display: cleaned,
        bookCode,
        chapter: 1,
        startVerse,
        endVerse,
      };
    }
  }

  const fullReferenceMatch = cleaned.match(
    /^([1-3]?\s*[A-Za-z]+(?:\s+[A-Za-z]+)*)\s+(\d+):(\d+)(?:-(\d+))?$/,
  );

  if (fullReferenceMatch) {
    const bookCode = matchBook(fullReferenceMatch[1], books);
    if (!bookCode) {
      return null;
    }

    const chapter = Number.parseInt(fullReferenceMatch[2], 10);
    const startVerse = Number.parseInt(fullReferenceMatch[3], 10);
    const endVerse = fullReferenceMatch[4]
      ? Number.parseInt(fullReferenceMatch[4], 10)
      : startVerse;

    if (
      Number.isNaN(chapter) ||
      Number.isNaN(startVerse) ||
      Number.isNaN(endVerse) ||
      endVerse < startVerse
    ) {
      return null;
    }

    return {
      display: cleaned,
      bookCode,
      chapter,
      startVerse,
      endVerse,
    };
  }

  if (!context) {
    return null;
  }

  const chapterVerseMatch = cleaned.match(/^(\d+):(\d+)(?:-(\d+))?$/);
  if (chapterVerseMatch) {
    const chapter = Number.parseInt(chapterVerseMatch[1], 10);
    const startVerse = Number.parseInt(chapterVerseMatch[2], 10);
    const endVerse = chapterVerseMatch[3]
      ? Number.parseInt(chapterVerseMatch[3], 10)
      : startVerse;

    if (
      Number.isNaN(chapter) ||
      Number.isNaN(startVerse) ||
      Number.isNaN(endVerse) ||
      endVerse < startVerse
    ) {
      return null;
    }

    return {
      display: cleaned,
      bookCode: context.bookCode,
      chapter,
      startVerse,
      endVerse,
    };
  }

  const verseOnlyMatch = cleaned.match(/^(\d+)(?:-(\d+))?$/);
  if (!verseOnlyMatch) {
    return null;
  }

  const startVerse = Number.parseInt(verseOnlyMatch[1], 10);
  const endVerse = verseOnlyMatch[2]
    ? Number.parseInt(verseOnlyMatch[2], 10)
    : startVerse;

  if (Number.isNaN(startVerse) || Number.isNaN(endVerse) || endVerse < startVerse) {
    return null;
  }

  return {
    display: cleaned,
    bookCode: context.bookCode,
    chapter: context.chapter,
    startVerse,
    endVerse,
  };
}

function parseReferences(display: string, books: BookInfo[]): ParsedReference[] | null {
  const segments = display
    .split(',')
    .map((segment) => segment.trim())
    .filter((segment) => segment.length > 0);

  if (segments.length === 0) {
    return null;
  }

  const parsedReferences: ParsedReference[] = [];
  let context: ReferenceContext | null = null;

  for (const segment of segments) {
    const parsed = parseSingleReferenceSegment(segment, books, context);
    if (!parsed) {
      return null;
    }

    parsedReferences.push(parsed);
    context = {
      bookCode: parsed.bookCode,
      chapter: parsed.chapter,
    };
  }

  return parsedReferences;
}

function sortTranslationNames(left: string, right: string): number {
  const leftPriority = PRIORITY_TRANSLATIONS.indexOf(left);
  const rightPriority = PRIORITY_TRANSLATIONS.indexOf(right);

  if (leftPriority !== -1 || rightPriority !== -1) {
    if (leftPriority === -1) return 1;
    if (rightPriority === -1) return -1;
    return leftPriority - rightPriority;
  }

  return left.localeCompare(right);
}

function formatReferenceLabel(bookCode: string, chapter: number, verseNumber: number): string {
  if (isThomasBookCode(bookCode)) {
    return `${bookCode} ${verseNumber}`;
  }
  return `${bookCode} ${chapter}:${verseNumber}`;
}

function stripCanonicalDisplayMarkers(text: string): string {
  return text
    .replace(/\[(?:vs|cr|fn):\s*\w+\]/gi, '')
    .replace(/\s{2,}/g, ' ')
    .trim();
}

function normalizeNewGospelVerse(result: NewGospelVerseQueryResult): NewGospelVerseRecord | null {
  const verse = result.queryNewGospelVerse?.[0];
  if (!verse?.uid || !verse?.number || !verse.text) {
    return null;
  }

  return {
    uid: verse.uid,
    number: verse.number,
    pericope: verse.pericope ?? null,
    text: verse.text,
    notes: verse.notes ?? null,
    assembledVerseMarks: verse.assembledVerseMarks ?? null,
    supportingReferences: (verse.supportingReferences ?? [])
      .flatMap((reference) => {
        const display = reference?.display?.trim();
        return display ? [{ display }] : [];
      }),
  };
}

function normalizeArchonVerse(result: ArchonVerseQueryResult): ArchonVerseRecord[] {
  return (result.queryVerse ?? [])
    .flatMap((verse) => {
      if (!verse?.number) {
        return [];
      }

      const verseContents = (verse.verseContents ?? [])
        .flatMap((content) => {
          if (
            content?.type !== 'text' ||
            !content.text ||
            !content.translation?.name
          ) {
            return [];
          }

          return [{
            text: content.text,
            translationName: content.translation.name,
            translationCode: content.translation.code ?? null,
            lemma: content.lemma ?? null,
            strong: content.strong ?? null,
            morph: content.morph ?? null,
            gloss: content.gloss ?? null,
            tokenIndex: typeof content.tokenIndex === 'number' ? content.tokenIndex : null,
          }];
        });

      const interlinearTokenMap = new Map<number, InterlinearToken>();

      for (const token of verse.interlinearTokens ?? []) {
        if (typeof token?.tokenIndex !== 'number') {
          continue;
        }

        interlinearTokenMap.set(token.tokenIndex, {
          tokenIndex: token.tokenIndex,
          strong: token.strong ?? null,
          reference: token.reference ?? null,
          transliteration: token.transliteration ?? null,
          original: token.original ?? null,
          englishGloss: token.englishGloss ?? null,
          grammar: token.grammar ?? null,
        });
      }

      for (const [alignmentIndex, alignment] of (verse.alignments ?? []).entries()) {
        if (!alignment) {
          continue;
        }

        const tokenIndex = typeof alignment?.wordIndex === 'number'
          ? alignment.wordIndex
          : alignmentIndex + 1;

        const current = interlinearTokenMap.get(tokenIndex) ?? {
          tokenIndex,
          strong: null,
          reference: null,
          transliteration: null,
          original: null,
          englishGloss: null,
          grammar: null,
        };

        interlinearTokenMap.set(tokenIndex, {
          ...current,
          strong: current.strong ?? alignment.lemma?.strongsId ?? null,
          transliteration: current.transliteration ?? alignment.lemma?.lemmaText ?? null,
          original: current.original ?? alignment.hebrewWord ?? null,
          englishGloss: current.englishGloss ?? alignment.englishGloss ?? alignment.lemma?.gloss ?? null,
        });
      }

      for (const [contentIndex, content] of verseContents.entries()) {
        const hasTokenMetadata = Boolean(
          content.strong || content.morph || content.gloss || content.lemma,
        );
        const isOriginalLanguage = ['WLC', 'SBLGNT', 'TR1894'].includes(content.translationCode ?? '');

        if (!hasTokenMetadata && !isOriginalLanguage) {
          continue;
        }

        const tokenIndex = typeof content.tokenIndex === 'number'
          ? content.tokenIndex
          : contentIndex + 1;

        const current = interlinearTokenMap.get(tokenIndex) ?? {
          tokenIndex,
          strong: null,
          reference: null,
          transliteration: null,
          original: null,
          englishGloss: null,
          grammar: null,
        };

        interlinearTokenMap.set(tokenIndex, {
          ...current,
          strong: current.strong ?? content.strong ?? null,
          transliteration: current.transliteration ?? content.lemma ?? null,
          original: current.original ?? content.text ?? null,
          englishGloss: current.englishGloss ?? content.gloss ?? null,
          grammar: current.grammar ?? content.morph ?? null,
        });
      }

      const interlinearTokens = [...interlinearTokenMap.values()]
        .sort((left, right) => left.tokenIndex - right.tokenIndex);

      return [{
        number: verse.number,
        verseContents,
        interlinearTokens,
      }];
    })
    .sort((left, right) => left.number - right.number);
}

async function fetchBooks(): Promise<BookInfo[]> {
  const result = await executeVardaRequest<BooksQueryResult>(
    'archondb',
    `
      query GetBooksForNewGospel {
        queryBook {
          id
          code
          nameEn
        }
      }
    `,
  );

  return (result.queryBook ?? [])
    .flatMap((book) => {
      if (!book?.code || !book?.nameEn) {
        return [];
      }

      return [{
        code: book.code,
        name: book.nameEn,
      }];
    });
}

async function fetchPericopeEntries(): Promise<PericopeVerseEntry[]> {
  const result = await executeVardaRequest<NewGospelVerseNumbersQueryResult>(
    'new_gospel',
    `
      query GetNewGospelPericopes {
        queryNewGospelVerse {
          number
          pericope
        }
      }
    `,
  );

  return (result.queryNewGospelVerse ?? [])
    .flatMap((verse) => {
      if (!verse?.number) {
        return [];
      }

      return [{
        number: verse.number,
        pericope: verse.pericope ?? null,
      }];
    })
    .sort((left, right) => left.number - right.number);
}

async function fetchComparisonMetricsForVerse(verseNumber: number): Promise<{
  metrics: ComparisonMetricRecord[];
  snapshot: ComparisonSnapshotInfo | null;
}> {
  const result = await executeVardaRequest<ComparisonVerseQueryResult>(
    'new_gospel',
    `
      query GetComparisonMetricsForVerse($verseNumber: Int!) {
        queryScriptureComparisonVerse(filter: { newGospelVerseNumber: { eq: $verseNumber } }) {
          snapshot {
            generatedAt
            thresholdPct
            verbatimRunFlagWords
          }
          metrics {
            comparisonKind
            rightTranslationCode
            ngramContainmentPct
            sequenceSimilarityPct
            tokenOverlapPct
            semanticSimilarityPct
            longestCommonRunWords
            longestCommonRunText
            baselineDeltaPct
          }
        }
      }
    `,
    { verseNumber },
  );

  const verses = result.queryScriptureComparisonVerse ?? [];
  if (verses.length === 0) {
    return { metrics: [], snapshot: null };
  }

  const latest = [...verses].sort((left, right) =>
    (right.snapshot?.generatedAt ?? '').localeCompare(left.snapshot?.generatedAt ?? ''),
  )[0];

  const snapshot: ComparisonSnapshotInfo | null = latest.snapshot?.thresholdPct != null && latest.snapshot?.verbatimRunFlagWords != null
    ? {
        thresholdPct: latest.snapshot.thresholdPct,
        verbatimRunFlagWords: latest.snapshot.verbatimRunFlagWords,
      }
    : null;

  const metrics = (latest.metrics ?? []).filter((metric) => metric.comparisonKind === 'local_vs_copyright');

  return { metrics, snapshot };
}

function buildPericopeTree(entries: PericopeVerseEntry[]): TreeNodeData[] {
  const grouped = new Map<string, PericopeVerseEntry[]>();

  for (const entry of entries) {
    const label = entry.pericope?.trim() || `Verse ${entry.number}`;
    const current = grouped.get(label) ?? [];
    current.push(entry);
    grouped.set(label, current);
  }

  return [...grouped.entries()].map(([pericopeLabel, pericopeEntries]) => {
    const firstVerse = pericopeEntries[0]?.number ?? 1;

    return {
      value: `pericope-${firstVerse}`,
      label: pericopeLabel,
      nodeProps: {
        verseNumber: firstVerse,
      },
      children: pericopeEntries.map((entry) => ({
        value: `pericope-${firstVerse}-verse-${entry.number}`,
        label: `Verse ${entry.number}`,
        nodeProps: {
          verseNumber: entry.number,
        },
      })),
    };
  });
}

function filterPericopeTree(nodes: TreeNodeData[], filterValue: string): TreeNodeData[] {
  const normalizedFilter = filterValue.trim().toLowerCase();

  if (!normalizedFilter) {
    return nodes;
  }

  return nodes.flatMap((node) => {
    const filteredChildren = node.children ? filterPericopeTree(node.children, filterValue) : [];
    const nodeLabel = typeof node.label === 'string' ? node.label.toLowerCase() : '';
    const matchesNode = nodeLabel.includes(normalizedFilter);

    if (matchesNode) {
      return [{
        ...node,
        children: node.children,
      }];
    }

    if (filteredChildren.length > 0) {
      return [{
        ...node,
        children: filteredChildren,
      }];
    }

    return [];
  });
}

async function fetchNewGospelVerse(verseNumber: number): Promise<NewGospelVerseRecord | null> {
  const result = await executeVardaRequest<NewGospelVerseQueryResult>(
    'new_gospel',
    `
      query GetNewGospelVerse($number: Int!) {
        queryNewGospelVerse(filter: { number: { eq: $number } }) {
          uid
          id
          number
          pericope
          text
          notes
          assembledVerseMarks
          supportingReferences {
            id
            display
          }
        }
      }
    `,
    { number: verseNumber },
  );

  return normalizeNewGospelVerse(result);
}

type UpdateNewGospelVerseResult = {
  updateNewGospelVerse?: boolean | null;
};

type VerseUpdate = Partial<Pick<NewGospelVerseRecord, 'text' | 'notes' | 'assembledVerseMarks'>>;

async function saveNewGospelVerse(uid: string, input: VerseUpdate): Promise<void> {
  const result = await executeVardaRequest<UpdateNewGospelVerseResult>(
    'new_gospel',
    `
      mutation UpdateNewGospelVerse($uid: ID!, $input: NewGospelVerseInput!) {
        updateNewGospelVerse(uid: $uid, input: $input)
      }
    `,
    { uid, input },
  );

  if (!result.updateNewGospelVerse) {
    throw new Error('Save failed.');
  }
}

function readVerseMarks(value: string | null): VerseMark[] {
  const marks: unknown = JSON.parse(value ?? '[]');
  if (!Array.isArray(marks) || !marks.every((mark) =>
    mark && typeof mark.id === 'string' && typeof mark.rowKey === 'string'
      && typeof mark.translation === 'string' && typeof mark.reference === 'string'
      && typeof mark.text === 'string' && Number.isInteger(mark.start)
      && Number.isInteger(mark.end) && mark.start >= 0 && mark.end > mark.start
  )) {
    throw new Error('Invalid saved verse marks.');
  }
  return marks;
}

function highlightedVerseText(row: LinkedVerseRow, marks: VerseMark[], deleteMark: (id: string) => void) {
  const ranges = marks.filter((mark) => mark.rowKey === row.key && mark.end <= row.text.length
    && row.text.slice(mark.start, mark.end) === mark.text);
  const boundaries = [...new Set([0, ...ranges.flatMap(({ start, end }) => [start, end]), row.text.length])]
    .sort((left, right) => left - right);

  return boundaries.slice(1).map((end, index) => {
    const start = boundaries[index];
    const text = row.text.slice(start, end);
    const coveringMarks = ranges.filter((mark) => mark.start <= start && mark.end >= end);
    if (!coveringMarks.length) {
      return text;
    }

    return (
      <mark key={start} className={styles.verseMark}>
        {text}
        <span className={styles.markActions}>
          {coveringMarks.map((mark) => (
            <button
              key={mark.id}
              type="button"
              className={styles.deleteMark}
              aria-label={`Delete mark: ${mark.text}`}
              title={`Delete mark: ${mark.text}`}
              onPointerDown={(event) => event.preventDefault()}
              onPointerUp={(event) => event.stopPropagation()}
              onClick={() => deleteMark(mark.id)}
            >
              <IconTrash size={14} aria-hidden="true" />
            </button>
          ))}
        </span>
      </mark>
    );
  });
}

function SortableMarkRow({ mark, index, count, onMove, onDelete }: {
  mark: VerseMark;
  index: number;
  count: number;
  onMove: (id: string, index: number) => void;
  onDelete: (id: string) => void;
}) {
  const { ref, handleRef } = useSortable({ id: mark.id, index });

  return (
    <Table.Tr ref={ref}>
      <Table.Td>
        <Group gap={4} wrap="nowrap">
          <button
            ref={handleRef}
            type="button"
            className={styles.markDragHandle}
            title="Drag to reorder"
            aria-label={`Reorder mark ${index + 1}`}
          >
            <IconGripVertical size={16} aria-hidden="true" />
          </button>
          {index + 1}
        </Group>
      </Table.Td>
      <Table.Td title={mark.reference}>{mark.translation}</Table.Td>
      <Table.Td><mark style={{ backgroundColor: '#ffeb66' }}>{mark.text}</mark></Table.Td>
      <Table.Td>
        <Group gap={4}>
          <Button size="compact-xs" variant="subtle" disabled={index === 0}
            aria-label={`Move mark ${index + 1} up`} onClick={() => onMove(mark.id, index - 1)}>↑</Button>
          <Button size="compact-xs" variant="subtle" disabled={index === count - 1}
            aria-label={`Move mark ${index + 1} down`} onClick={() => onMove(mark.id, index + 1)}>↓</Button>
          <Button size="compact-xs" variant="subtle" color="red"
            aria-label={`Delete mark ${index + 1}`} onClick={() => onDelete(mark.id)}>Delete</Button>
        </Group>
      </Table.Td>
    </Table.Tr>
  );
}

async function fetchArchonVerses(reference: ParsedReference): Promise<ArchonVerseRecord[]> {
  const verseQueries = [];

  for (let verseNumber = reference.startVerse; verseNumber <= reference.endVerse; verseNumber += 1) {
    verseQueries.push(
      executeVardaRequest<ArchonVerseQueryResult>(
        'archondb',
        `
          query GetArchonVerseForNewGospel($bookCode: String!, $chapter: Int!, $verse: Int!) {
            queryVerse(filter: {
              chunkType: { eq: "single_verse" }
              number: { eq: $verse }
              chapter: {
                number: { eq: $chapter }
                bookTranslation: {
                  book: {
                    code: { eq: $bookCode }
                  }
                }
              }
            }) {
              id
              number
              verseContents {
                id
                text
                type
                lemma
                strong
                morph
                gloss
                tokenIndex
                translation {
                  id
                  code
                  name
                }
              }
              interlinearTokens {
                tokenIndex
                strong
                reference
                transliteration
                original
                englishGloss
                grammar
              }
              alignments {
                wordIndex
                hebrewWord
                englishGloss
                lemma {
                  strongsId
                  lemmaText
                  gloss
                }
              }
            }
          }
        `,
        {
          bookCode: reference.bookCode,
          chapter: reference.chapter,
          verse: verseNumber,
        },
      ),
    );
  }

  const results = await Promise.all(verseQueries);
  const verses: ArchonVerseRecord[] = [];

  for (const result of results) {
    verses.push(...normalizeArchonVerse(result));
  }

  return verses.sort((left, right) => left.number - right.number);
}

async function fetchInterlinearVerse(reference: InterlinearReference): Promise<InterlinearRow[]> {
  const result = await executeVardaRequest<FixedInterlinearQueryResult>(
    'archondb',
    `
      query GetInterlinearVerseForNewGospel($bookCode: String!, $chapter: Int!, $verse: Int!) {
        queryVerse(filter: {
          chunkType: { eq: "single_verse" }
          number: { eq: $verse }
          chapter: {
            number: { eq: $chapter }
            bookTranslation: {
              book: {
                code: { eq: $bookCode }
              }
              translation: {
                code: { eq: "interlinear" }
              }
            }
          }
        }) {
          interlinearTokens {
            tokenIndex
            strong
            reference
            transliteration
            original
            englishGloss
            grammar
          }
          alignments {
            wordIndex
            hebrewWord
            englishGloss
            lemma {
              strongsId
              lemmaText
              gloss
            }
          }
        }
      }
    `,
    {
      bookCode: reference.bookCode,
      chapter: reference.chapter,
      verse: reference.verseNumber,
    },
  );

  const verse = result.queryVerse?.[0];
  const tokenMap = new Map<number, InterlinearToken>();

  for (const token of verse?.interlinearTokens ?? []) {
    if (typeof token?.tokenIndex !== 'number') {
      continue;
    }

    tokenMap.set(token.tokenIndex, {
      tokenIndex: token.tokenIndex,
      strong: token.strong ?? null,
      reference: token.reference ?? null,
      transliteration: token.transliteration ?? null,
      original: token.original ?? null,
      englishGloss: token.englishGloss ?? null,
      grammar: token.grammar ?? null,
    });
  }

  for (const [index, alignment] of (verse?.alignments ?? []).entries()) {
    if (!alignment) {
      continue;
    }

    const tokenIndex = typeof alignment.wordIndex === 'number' ? alignment.wordIndex : index + 1;
    const current = tokenMap.get(tokenIndex) ?? {
      tokenIndex,
      strong: null,
      reference: null,
      transliteration: null,
      original: null,
      englishGloss: null,
      grammar: null,
    };

    tokenMap.set(tokenIndex, {
      ...current,
      strong: current.strong ?? alignment.lemma?.strongsId ?? null,
      transliteration: current.transliteration ?? alignment.lemma?.lemmaText ?? null,
      original: current.original ?? alignment.hebrewWord ?? null,
      englishGloss: current.englishGloss ?? alignment.englishGloss ?? alignment.lemma?.gloss ?? null,
    });
  }

  const tokens = [...tokenMap.values()].sort((left, right) => left.tokenIndex - right.tokenIndex);
  if (tokens.length === 0) {
    return [];
  }

  return [{
    key: `interlinear-${reference.bookCode}-${reference.chapter}-${reference.verseNumber}`,
    translationName: 'Interlinear',
    display: formatReferenceLabel(reference.bookCode, reference.chapter, reference.verseNumber),
    referenceLabel: formatReferenceLabel(reference.bookCode, reference.chapter, reference.verseNumber),
    tokens,
  }];
}

async function buildLinkedVerseRows(
  references: SupportingReferenceDisplay[],
  books: BookInfo[],
): Promise<{ rows: LinkedVerseRow[]; interlinearRows: InterlinearRow[]; parseErrors: string[] }> {
  const parseErrors: string[] = [];
  const grouped = new Map<string, LinkedVerseRow[]>();
  const interlinearRows: InterlinearRow[] = [];

  for (const reference of references) {
    const parsedReferences = parseReferences(reference.display, books);
    if (!parsedReferences) {
      parseErrors.push(reference.display);
      continue;
    }

    for (const parsedReference of parsedReferences) {
      const isThomas = isThomasBookCode(parsedReference.bookCode);
      if (!isThomas) {
        const interlinearVerseRows = await Promise.all(
          Array.from(
            { length: parsedReference.endVerse - parsedReference.startVerse + 1 },
            (_, offset) => fetchInterlinearVerse({
              bookCode: parsedReference.bookCode,
              chapter: parsedReference.chapter,
              verseNumber: parsedReference.startVerse + offset,
            }),
          ),
        );
        interlinearRows.push(...interlinearVerseRows.flat());
      }

      const verses = await fetchArchonVerses(parsedReference);
      for (const verse of verses) {
        const referenceLabel = formatReferenceLabel(
          parsedReference.bookCode,
          parsedReference.chapter,
          verse.number,
        );

        if (isThomas && verse.interlinearTokens.length > 0) {
          interlinearRows.push({
            key: `interlinear-thomas-${reference.display}-${parsedReference.bookCode}-${parsedReference.chapter}-${verse.number}`,
            translationName: 'Nag Hammadi Coptic Text',
            display: reference.display,
            referenceLabel,
            tokens: verse.interlinearTokens,
          });
        }

        for (const content of verse.verseContents) {
          if (content.translationName === 'Interlinear') {
            continue;
          }
          const { translationCode, translationName } = content;
          if (isThomas) {
            if (isNagHammadiCopticTranslation(translationName)
              || isThomasCopticMachineTranslation(translationName)
              || EXCLUDED_TRANSLATIONS.includes(translationCode ?? '')
              || EXCLUDED_TRANSLATIONS.includes(translationName)) {
              continue;
            }
          } else {
            const code = translationCode?.trim();
            const selected = code
              ? BIBLE_TRANSLATIONS.has(code.toUpperCase())
              : BIBLE_TRANSLATION_NAMES.has(translationName);
            if (!selected) continue;
          }

          const currentRows = grouped.get(content.translationName) ?? [];
          currentRows.push({
            key: `${content.translationName}-${reference.display}-${parsedReference.bookCode}-${parsedReference.chapter}-${verse.number}-${currentRows.length}`,
            translationName: content.translationName,
            display: reference.display,
            referenceLabel,
            verseNumber: verse.number,
            text: stripCanonicalDisplayMarkers(content.text),
          });
          grouped.set(content.translationName, currentRows);
        }
      }
    }
  }

  const sortedTranslations = [...grouped.keys()].sort(sortTranslationNames);
  const rows = sortedTranslations.flatMap((translationName) => grouped.get(translationName) ?? []);

  return { rows, interlinearRows, parseErrors };
}

export function NewGospelPage() {
  const [books, setBooks] = useState<BookInfo[]>([]);
  const [lastVerseNumber, setLastVerseNumber] = useState<number>(1);
  const { currentVerseNumber, setCurrentVerseNumber } = useGospelStore();
  const [drawerOpened, setDrawerOpened] = useState<boolean>(false);
  const [filterInput, setFilterInput] = useState('');
  const [debouncedFilter, setDebouncedFilter] = useDebouncedState('', 200);
  const [pericopeTree, setPericopeTree] = useState<TreeNodeData[]>([]);
  const [verse, setVerse] = useState<NewGospelVerseRecord | null>(null);
  const [verseDraft, setVerseDraft] = useState('');
  const [notesDraft, setNotesDraft] = useState('');
  const [rows, setRows] = useState<LinkedVerseRow[]>([]);
  const [interlinearRows, setInterlinearRows] = useState<InterlinearRow[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [marks, setMarks] = useState<VerseMark[]>([]);
  const [marking, setMarking] = useState(false);
  const [showResult, setShowResult] = useState(false);
  const pendingSave = useRef<Promise<void> | null>(null);
  const [saving, setSaving] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [saveMessage, setSaveMessage] = useState<string | null>(null);
  const [parseErrors, setParseErrors] = useState<string[]>([]);
  const [comparisonMetrics, setComparisonMetrics] = useState<ComparisonMetricRecord[]>([]);
  const [recalculating, setRecalculating] = useState<boolean>(false);

  function handleRecalc() {
    if (!verse || recalculating || comparisonMetrics.length === 0) return;

    const firstRef = verse.supportingReferences?.[0]?.display ?? '';
    const match = firstRef.match(/^(?<book>[1-3]?\s*[A-Za-z]+(?:\s+[A-Za-z]+)*)\s+(?<chapter>\d+):(?<verse>\d+)/);
    if (!match?.groups) return;

    const bookName = match.groups.book.trim();
    const chapter = parseInt(match.groups.chapter, 10);
    const verseNumber = parseInt(match.groups.verse, 10);

    const bookCode = matchBook(bookName, books);
    if (!bookCode) {
      console.error('Unknown book:', bookName);
      return;
    }

    const translationCodes = comparisonMetrics
      .map((m) => m.rightTranslationCode)
      .filter((code): code is string => code != null);

    if (translationCodes.length === 0) return;

    setRecalculating(true);

    fetch('http://localhost:9001/compare/batch', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        local_text: verseDraft,
        book_code: bookCode,
        chapter,
        verse_number: verseNumber,
        translation_codes: translationCodes,
      }),
    })
      .then(async (res) => {
        const results: Array<{
          translation_code: string;
          ngram_containment_pct: number;
          sequence_similarity_pct: number;
          token_overlap_pct: number;
          longest_common_run_words: number;
          longest_common_run_text: string;
        }> = await res.json();

        if (res.ok) {
          const resultMap = new Map(results.map((r) => [r.translation_code, r]));

          setComparisonMetrics((prev) =>
            prev.map((m) => {
              const result = resultMap.get(m.rightTranslationCode ?? '');
                return result
                ? {
                    ...m,
                    ngramContainmentPct: result.ngram_containment_pct,
                    sequenceSimilarityPct: result.sequence_similarity_pct,
                    tokenOverlapPct: result.token_overlap_pct,
                    longestCommonRunWords: result.longest_common_run_words,
                    longestCommonRunText: result.longest_common_run_text,
                  }
                : m;
            }),
          );
        }
      })
      .catch(console.error)
      .finally(() => setRecalculating(false));
  }
  const tree = useTree();
  const filteredPericopeTree = filterPericopeTree(pericopeTree, debouncedFilter);

  useEffect(() => {
    let cancelled = false;

    async function loadMeta() {
      try {
        setLoading(true);
        setError(null);

        const [bookData, pericopeEntries] = await Promise.all([
          fetchBooks(),
          fetchPericopeEntries(),
        ]);

        if (cancelled) {
          return;
        }

        setBooks(bookData);
        setLastVerseNumber(pericopeEntries.length > 0 ? pericopeEntries[pericopeEntries.length - 1].number : 1);
        setPericopeTree(buildPericopeTree(pericopeEntries));
      } catch (loadError) {
        if (!cancelled) {
          console.error(loadError);
          setError('Failed to load the new_gospel route metadata.');
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    void loadMeta();

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (books.length === 0) {
      return;
    }

    let cancelled = false;

    async function loadVerse() {
      try {
        setLoading(true);
        setError(null);
        setParseErrors([]);

        const currentVerse = await fetchNewGospelVerse(currentVerseNumber);
        if (!currentVerse) {
          throw new Error(`Verse ${currentVerseNumber} was not found in new_gospel.`);
        }

        const [linkedVerses, comparisonData] = await Promise.all([
          buildLinkedVerseRows(currentVerse.supportingReferences, books),
          fetchComparisonMetricsForVerse(currentVerseNumber),
        ]);

        if (cancelled) {
          return;
        }

        setMarks(readVerseMarks(currentVerse.assembledVerseMarks));
        setMarking(false);
        setShowResult(false);
        setVerse(currentVerse);
        setVerseDraft(currentVerse.text);
        setNotesDraft(currentVerse.notes ?? '');
        setSaveMessage(null);
        setRows(linkedVerses.rows);
        setInterlinearRows(linkedVerses.interlinearRows);
        setParseErrors(linkedVerses.parseErrors);
        setComparisonMetrics(comparisonData.metrics);
      } catch (loadError) {
        if (!cancelled) {
          console.error(loadError);
          setError('Failed to load the selected new_gospel verse.');
          setVerse(null);
          setRows([]);
          setInterlinearRows([]);
          setParseErrors([]);
          setComparisonMetrics([]);
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    void loadVerse();

    return () => {
      cancelled = true;
    };
  }, [books, currentVerseNumber]);

  const hasLinkedVerses = rows.length > 0;
  const hasInterlinearRows = interlinearRows.length > 0;
  const serializedMarks = JSON.stringify(marks);
  const marksDirty = verse !== null && serializedMarks !== (verse.assembledVerseMarks ?? '[]');
  const draftDirty = verse !== null
    && (verseDraft !== verse.text || notesDraft !== (verse.notes ?? ''));
  const navigationBlocked = loading || saving || marksDirty;

  function queueVerseSave(uid: string, input: VerseUpdate, manual = false) {
    setSaving(true);
    setSaveMessage(null);
    const save = (pendingSave.current ?? Promise.resolve()).then(async () => {
      try {
        await saveNewGospelVerse(uid, input);
        setVerse((current) => current?.uid === uid ? { ...current, ...input } : current);
        setError(null);
        if (manual) {
          setSaveMessage('Saved.');
        }
      } catch (saveError) {
        console.error(saveError);
        setError(input.assembledVerseMarks !== undefined
          ? 'Verse marks are unsaved. Use Save to retry.'
          : 'Failed to save the selected new_gospel verse.');
      }
    }).finally(() => {
      if (pendingSave.current === save) {
        pendingSave.current = null;
        setSaving(false);
      }
    });
    pendingSave.current = save;
  }

  function handleSave() {
    if (!verse || pendingSave.current || loading) {
      return;
    }
    const input: VerseUpdate = {};
    if (draftDirty) {
      input.text = verseDraft;
      input.notes = notesDraft.trim() ? notesDraft : null;
    }
    if (marksDirty) {
      input.assembledVerseMarks = serializedMarks;
    }
    queueVerseSave(verse.uid, input, true);
  }

  function updateMarks(next: VerseMark[]) {
    if (verse && !loading) {
      setMarks(next);
      queueVerseSave(verse.uid, { assembledVerseMarks: JSON.stringify(next) });
    }
  }

  function captureMark(row: LinkedVerseRow, element: HTMLElement) {
    if (!marking || showResult || loading) {
      return;
    }
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount !== 1) {
      return;
    }
    const range = selection.getRangeAt(0);
    if (!element.contains(range.startContainer) || !element.contains(range.endContainer)) {
      return;
    }
    const prefix = document.createRange();
    prefix.selectNodeContents(element);
    prefix.setEnd(range.startContainer, range.startOffset);
    const selected = range.toString();
    const text = selected.trim();
    const start = prefix.toString().length + selected.length - selected.trimStart().length;
    const end = start + text.length;
    if (!text || row.text.slice(start, end) !== text) {
      return;
    }
    selection.removeAllRanges();
    updateMarks([...marks, {
      id: Array.from(crypto.getRandomValues(new Uint32Array(4)), (value) => value.toString(16).padStart(8, '0')).join(''),
      rowKey: row.key,
      translation: row.translationName,
      reference: row.referenceLabel,
      text,
      start,
      end,
    }]);
    setMarking(false);
  }

  function deleteMark(id: string) {
    updateMarks(marks.filter((mark) => mark.id !== id));
  }

  function moveMark(id: string, destination: number) {
    const from = marks.findIndex((mark) => mark.id === id);
    if (from < 0 || from === destination || destination < 0 || destination >= marks.length) {
      return;
    }
    const next = [...marks];
    const [mark] = next.splice(from, 1);
    next.splice(destination, 0, mark);
    updateMarks(next);
  }

  const visibleRows = hasInterlinearRows
    ? rows.filter((row) => row.translationName !== 'Interlinear')
    : rows;

  const tableRows = visibleRows.map((row, index) => {
    const previousRow = index > 0 ? visibleRows[index - 1] : null;
    const showTranslation = !previousRow || previousRow.translationName !== row.translationName;

    return (
      <Table.Tr key={row.key}>
        <Table.Td style={{ width: FIRST_COLUMN_WIDTH_PX, minWidth: FIRST_COLUMN_WIDTH_PX }}>
          {showTranslation ? <Text fw={600}>{row.translationName}</Text> : null}
        </Table.Td>
        <Table.Td>
          <Text span fw={600}>{row.referenceLabel}</Text>
          {' '}
          <Text
            span
            style={{ cursor: marking ? 'text' : undefined }}
            onPointerUp={(event) => captureMark(row, event.currentTarget)}
            onKeyUp={(event) => {
              if (event.key === 'Shift') {
                captureMark(row, event.currentTarget);
              }
            }}
          >
            {highlightedVerseText(row, marks, deleteMark)}
          </Text>
        </Table.Td>
      </Table.Tr>
    );
  });

  const interlinearBlocks = interlinearRows.map((row, rowIndex) => {
    const previousRow = rowIndex > 0 ? interlinearRows[rowIndex - 1] : null;
    const showTranslation = !previousRow || previousRow.translationName !== row.translationName;

    return (
      <div
        key={row.key}
        style={{
          display: 'flex',
          borderBottom: '1px solid var(--mantine-color-gray-3)',
        }}
      >
        <div style={{ width: FIRST_COLUMN_WIDTH_PX, minWidth: FIRST_COLUMN_WIDTH_PX, padding: '10px 12px', fontWeight: 600 }}>
          {showTranslation ? row.translationName : null}
        </div>
        <div style={{ flex: 1, minWidth: 0, paddingTop: 10, paddingBottom: 14 }}>
          <div style={{ overflowX: 'auto', overflowY: 'hidden' }}>
            <div style={{ display: 'inline-flex', alignItems: 'flex-start', gap: 18, minWidth: 'max-content', paddingRight: 12 }}>
              {row.tokens.map((token) => {
                const strongLabel = token.strong?.trim() ?? '';
                const referenceLabel = token.reference?.trim()
                  ? `[${token.reference.trim()}]`
                  : '';
                const topLabel = [strongLabel, referenceLabel].filter(Boolean).join(' ');

                return (
                  <div
                    key={`${row.key}-${token.tokenIndex}`}
                    style={{
                      minWidth: 72,
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 4,
                      whiteSpace: 'nowrap',
                    }}
                  >
                    <Text size="sm" c="blue" fw={500}>
                      {topLabel || '\u00A0'}
                    </Text>
                    <Text size="1.1rem" c="blue" fw={500} style={{ lineHeight: 1.15 }}>
                      {token.transliteration || '\u00A0'}
                    </Text>
                    <Text
                      ff="serif"
                      size="1.75rem"
                      style={{ lineHeight: 1, color: 'var(--mantine-color-dark-9)' }}
                    >
                      {token.original || '\u00A0'}
                    </Text>
                    <Text c="orange.7" fw={500} size="1rem" style={{ lineHeight: 1.1 }}>
                      {token.englishGloss || '\u00A0'}
                    </Text>
                    <Text size="sm" c="blue" style={{ lineHeight: 1.1 }}>
                      {token.grammar || '\u00A0'}
                    </Text>
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      </div>
    );
  });

  return (
    <Tabs defaultValue="new_gospel" style={{ flex: 1, minHeight: 0, height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <Tabs.List>
        <Tabs.Tab value="new_gospel">Gospel</Tabs.Tab>
        <Tabs.Tab value="comparison">Comparison</Tabs.Tab>
      </Tabs.List>

      <Tabs.Panel value="new_gospel" style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <Container
          size="100%"
          px={0}
          style={{ flex: 1, minHeight: 0, height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}
        >
      <Drawer
        opened={drawerOpened}
        onClose={() => {
          setDrawerOpened(false);
          setFilterInput('');
          setDebouncedFilter('');
        }}
        title="Pericopes"
        position="right"
        size="md"
      >
        <Stack gap="sm">
          <TextInput
            placeholder="Filter pericopes"
            value={filterInput}
            onChange={(event) => {
              const nextValue = event.currentTarget.value;
              setFilterInput(nextValue);
              setDebouncedFilter(nextValue);
            }}
          />

          <Tree
            data={filteredPericopeTree}
            tree={tree}
            selectOnClick
            expandOnClick
            renderNode={({ node, elementProps, hasChildren }) => (
              <Group
                gap="xs"
                {...elementProps}
                onClick={(event) => {
                  elementProps.onClick(event);

                  if (!hasChildren) {
                    const verseNumber = node.nodeProps?.verseNumber;
                    if (typeof verseNumber === 'number' && !navigationBlocked && !pendingSave.current) {
                      setCurrentVerseNumber(verseNumber);
                      setDrawerOpened(false);
                    }
                  }
                }}
                style={{
                  ...elementProps.style,
                  cursor: 'pointer',
                  width: '100%',
                  paddingTop: 4,
                  paddingBottom: 4,
                }}
              >
                <Text size="sm">{node.label}</Text>
              </Group>
            )}
          />
        </Stack>
      </Drawer>

      <Stack gap="lg" style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
        <Stack gap="xs">
          <Group justify="space-between" align="center">
            <Group gap="sm" align="center">
              <Text c="dimmed">
                Verse {currentVerseNumber} of {lastVerseNumber}
              </Text>
              <Button
                variant="light"
                size="compact-sm"
                onClick={() => setDrawerOpened(true)}
              >
                B
              </Button>
              {comparisonMetrics.length > 0 ? (
                <>
                  {comparisonMetrics
                    .sort((left, right) => (left.rightTranslationCode ?? '').localeCompare(right.rightTranslationCode ?? ''))
                    .map((metric) => {
                      const hasRun = metric.longestCommonRunWords != null;
                      const tooltipLabel = hasRun
                        ? `Ng Cont ${formatPct(metric.ngramContainmentPct)} | Run ${metric.longestCommonRunWords} words | "${metric.longestCommonRunText ?? ''}"`
                        : undefined;

                      const badge = (
                        <Badge
                          variant="filled"
                          style={{
                            background: scoreTone(metric.ngramContainmentPct ?? 0),
                            color: '#f7f3ea',
                          }}
                        >
                          {metric.rightTranslationCode} {Math.round(metric.ngramContainmentPct ?? 0)}%
                        </Badge>
                      );

                      return tooltipLabel ? (
                        <Tooltip key={metric.rightTranslationCode} label={tooltipLabel} multiline>
                          {badge}
                        </Tooltip>
                      ) : (
                        <span key={metric.rightTranslationCode}>{badge}</span>
                      );
                    })}
                  <Button
                    variant="filled"
                    color="blue"
                    radius="xl"
                    size="compact-sm"
                    onClick={() => void handleRecalc()}
                    disabled={recalculating}
                  >
                    {recalculating ? '...' : 'Recalc'}
                  </Button>
                </>
              ) : null}
            </Group>
            <Button onClick={() => void handleSave()} disabled={(!draftDirty && !marksDirty) || loading || saving}>
              {saving ? 'Saving...' : 'Save'}
            </Button>
          </Group>
        </Stack>

        {error ? (
          <Alert color="red" title="Error">
            {error}
          </Alert>
        ) : null}

        {parseErrors.length > 0 ? (
          <Alert color="yellow" title="Unsupported references">
            {parseErrors.join(', ')}
          </Alert>
        ) : null}

        {saveMessage ? (
          <Alert color="green" title="Saved">
            {saveMessage}
          </Alert>
        ) : null}

        {!loading && verse ? (
          <Textarea
            label={verse.pericope || 'Verse'}
            value={verseDraft}
            onChange={(event) => setVerseDraft(event.currentTarget.value)}
            autosize
            minRows={4}
          />
        ) : null}

        {!loading && verse ? (
          <Textarea
            label="Notes"
            value={notesDraft}
            onChange={(event) => setNotesDraft(event.currentTarget.value)}
            autosize
            minRows={4}
          />
        ) : null}

        {!loading && verse ? (
          <Group gap="xs">
            <Button
              color="yellow"
              variant={marking ? 'filled' : 'light'}
              leftSection={<IconHighlight size={16} />}
              aria-pressed={marking}
              disabled={showResult || rows.length === 0}
              onClick={() => setMarking((active) => !active)}
            >
              Mark
            </Button>
            <Button onClick={() => {
              setShowResult((visible) => !visible);
              setMarking(false);
            }}>
              {showResult ? 'Show Verses' : 'Show Result'}
            </Button>
          </Group>
        ) : null}

        <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
          {loading ? (
            <Group justify="center" py="xl">
              <Loader />
            </Group>
          ) : null}

          {!loading && verse ? (
            showResult ? (
              <Stack gap="sm" p="sm" style={{ height: '100%', overflow: 'auto', border: '1px solid var(--mantine-color-gray-3)' }}>
                <Title order={4}>Assembled verse</Title>
                {marks.length === 0 ? (
                  <Text c="dimmed">No phrases marked yet.</Text>
                ) : (
                  <>
                    <Text size="lg" fw={600}>{marks.map((mark) => mark.text).join(' ')}</Text>
                    <DragDropProvider onDragEnd={({ canceled, operation: { source } }) => {
                      if (!canceled && isSortable(source) && typeof source.id === 'string') {
                        moveMark(source.id, source.index);
                      }
                    }}>
                      <Table striped aria-label="Marking order">
                        <Table.Thead>
                          <Table.Tr>
                            <Table.Th>Order</Table.Th>
                            <Table.Th>Translation</Table.Th>
                            <Table.Th>Marked phrase</Table.Th>
                            <Table.Th>Actions</Table.Th>
                          </Table.Tr>
                        </Table.Thead>
                        <Table.Tbody>
                          {marks.map((mark, index) => (
                            <SortableMarkRow
                              key={mark.id}
                              mark={mark}
                              index={index}
                              count={marks.length}
                              onMove={moveMark}
                              onDelete={deleteMark}
                            />
                          ))}
                        </Table.Tbody>
                      </Table>
                      <DragOverlay>
                        {(source) => {
                          const index = marks.findIndex((mark) => mark.id === source.id);
                          const mark = marks[index];
                          return mark && (
                            <Group className={styles.markDragPreview} gap="sm" aria-hidden="true">
                              <IconGripVertical size={16} />
                              <Text span>{index + 1}</Text>
                              <Text span c="dimmed">{mark.translation}</Text>
                              <mark className={styles.verseMark}>{mark.text}</mark>
                            </Group>
                          );
                        }}
                      </DragOverlay>
                    </DragDropProvider>
                  </>
                )}
              </Stack>
            ) : hasLinkedVerses ? (
              <div style={{ height: '100%', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
                <Group
                  gap={0}
                  px="sm"
                  pb="xs"
                  style={{
                    borderBottom: '1px solid var(--mantine-color-gray-3)',
                    fontWeight: 600,
                  }}
                >
                  <div style={{ width: FIRST_COLUMN_WIDTH_PX, minWidth: FIRST_COLUMN_WIDTH_PX }}>Translation</div>
                  <div style={{ flex: 1 }}>Verse</div>
                </Group>
                <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden' }}>
                  {hasInterlinearRows ? (
                    <div
                      style={{
                        borderLeft: '1px solid var(--mantine-color-gray-3)',
                        borderRight: '1px solid var(--mantine-color-gray-3)',
                        borderTop: '1px solid var(--mantine-color-gray-3)',
                        marginBottom: 8,
                      }}
                    >
                      {interlinearBlocks}
                    </div>
                  ) : null}
                  <Table striped highlightOnHover>
                    <Table.Tbody>{tableRows}</Table.Tbody>
                  </Table>
                </div>
              </div>
            ) : (
              hasInterlinearRows ? (
                <div style={{ height: '100%', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
                  <Group
                    gap={0}
                    px="sm"
                    pb="xs"
                    style={{
                      borderBottom: '1px solid var(--mantine-color-gray-3)',
                      fontWeight: 600,
                    }}
                  >
                    <div style={{ width: FIRST_COLUMN_WIDTH_PX, minWidth: FIRST_COLUMN_WIDTH_PX }}>Translation</div>
                    <div style={{ flex: 1 }}>Verse</div>
                  </Group>
                  <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden' }}>
                    <div
                      style={{
                        borderLeft: '1px solid var(--mantine-color-gray-3)',
                        borderRight: '1px solid var(--mantine-color-gray-3)',
                        borderTop: '1px solid var(--mantine-color-gray-3)',
                      }}
                    >
                      {interlinearBlocks}
                    </div>
                  </div>
                </div>
              ) : (
                <Alert color="blue" title="No linked verses">
                  No Archon verses were found for this entry&apos;s supporting references.
                </Alert>
              )
            )
          ) : null}
        </div>

        <Group justify="space-between" grow>
          <Button
            variant="default"
            fullWidth
            disabled={currentVerseNumber <= 1 || navigationBlocked}
            onClick={() => setCurrentVerseNumber((current) => Math.max(1, current - 1))}
          >
            Previous
          </Button>
          <Button
            fullWidth
            disabled={currentVerseNumber >= lastVerseNumber || navigationBlocked}
            onClick={() => setCurrentVerseNumber((current) => Math.min(lastVerseNumber, current + 1))}
          >
            Next
          </Button>
        </Group>
      </Stack>
        </Container>
      </Tabs.Panel>

      <Tabs.Panel value="comparison" style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        <ComparisonPage />
      </Tabs.Panel>
    </Tabs>
  );
}
