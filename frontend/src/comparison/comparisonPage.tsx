import { useEffect, useMemo, useState } from 'react';
import {
  Alert,
  Badge,
  Card,
  Container,
  Drawer,
  Group,
  Loader,
  Paper,
  ScrollArea,
  SimpleGrid,
  Stack,
  Table,
  Tabs,
  Text,
  Title,
} from '@mantine/core';
import { executeVardaRequest } from '../lib/client';

function stripCanonicalDisplayMarkers(text: string): string {
  return text
    .replace(/\[(?:vs|cr|fn):\s*\w+\]/gi, '')
    .replace(/\s{2,}/g, ' ')
    .trim();
}

type SnapshotQueryResult = {
  queryScriptureComparisonSnapshot?: SnapshotRecord[] | null;
};

type SnapshotRecord = {
  id?: string | null;
  generatedAt?: string | null;
  status?: string | null;
  books?: string[] | null;
  translationCodes?: string[] | null;
  thresholdMetric?: string | null;
  verbatimRunFlagWords?: number | null;
  thresholdBufferPct?: number | null;
  baselineMeanPct?: number | null;
  thresholdPct?: number | null;
  totalCopyrightedVerses?: number | null;
  matchedLocalVerses?: number | null;
  unmatchedCopyrightedVerses?: number | null;
  parseErrorCount?: number | null;
  pairSummaries?: PairSummaryRecord[] | null;
  histogramBuckets?: HistogramBucketRecord[] | null;
  verses?: ComparisonVerseRecord[] | null;
};

type PairSummaryRecord = {
  wordRunCounts?: number[] | null;
  comparisonKind?: string | null;
  leftTranslationCode?: string | null;
  rightTranslationCode?: string | null;
  averageNgramContainmentPct?: number | null;
  averageSequenceSimilarityPct?: number | null;
  minSequenceSimilarityPct?: number | null;
  maxSequenceSimilarityPct?: number | null;
  verseCount?: number | null;
};

type HistogramBucketRecord = {
  bucketStart?: number | null;
  bucketEnd?: number | null;
  count?: number | null;
};

type ComparisonVerseRecord = {
  id?: string | null;
  reference?: string | null;
  bookCode?: string | null;
  bookName?: string | null;
  chapter?: number | null;
  verse?: number | null;
  newGospelVerseNumber?: number | null;
  newGospelVerseUid?: string | null;
  supportingReferenceDisplay?: string | null;
  matchOrdinal?: number | null;
  hasLocalMatch?: boolean | null;
  parseError?: boolean | null;
  parseErrorReason?: string | null;
  texts?: VerseTextRecord[] | null;
  metrics?: MetricRecord[] | null;
};

type VerseTextRecord = {
  translationCode?: string | null;
  translationName?: string | null;
  sourceKind?: string | null;
  text?: string | null;
};

type MetricRecord = {
  comparisonKind?: string | null;
  leftTranslationCode?: string | null;
  rightTranslationCode?: string | null;
  sequenceSimilarityPct?: number | null;
  ngramContainmentPct?: number | null;
  tokenOverlapPct?: number | null;
  semanticSimilarityPct?: number | null;
  longestCommonRunWords?: number | null;
  longestCommonRunText?: string | null;
  baselineDeltaPct?: number | null;
  exceedsThreshold?: boolean | null;
};

type SnapshotViewModel = {
  generatedAt: string;
  books: string[];
  translationCodes: string[];
  verbatimRunFlagWords: number;
  thresholdPct: number;
  totalCopyrightedVerses: number;
  matchedLocalVerses: number;
  unmatchedCopyrightedVerses: number;
  parseErrorCount: number;
  flaggedVerseCount: number;
  pairSummaries: PairSummaryRecord[];
  histogramBuckets: HistogramBucketRecord[];
  verses: ComparisonVerseRecord[];
};

async function fetchSnapshot(): Promise<SnapshotViewModel | null> {
  const query = (includeCounts: boolean) => `
      query GetComparisonSnapshot {
        queryScriptureComparisonSnapshot {
          id
          generatedAt
          status
          books
          translationCodes
          thresholdMetric
          verbatimRunFlagWords
          thresholdBufferPct
          thresholdPct
          totalCopyrightedVerses
          matchedLocalVerses
          unmatchedCopyrightedVerses
          parseErrorCount
          pairSummaries {
            ${includeCounts ? 'wordRunCounts' : ''}
            comparisonKind
            leftTranslationCode
            rightTranslationCode
            averageNgramContainmentPct
            averageSequenceSimilarityPct
            minSequenceSimilarityPct
            maxSequenceSimilarityPct
            verseCount
          }
          histogramBuckets {
            bucketStart
            bucketEnd
            count
          }
          verses {
            id
            reference
            bookCode
            bookName
            chapter
            verse
            newGospelVerseNumber
            newGospelVerseUid
            supportingReferenceDisplay
            matchOrdinal
            hasLocalMatch
            parseError
            parseErrorReason
            texts {
              translationCode
              translationName
              sourceKind
              text
            }
            metrics {
              comparisonKind
              leftTranslationCode
              rightTranslationCode
              sequenceSimilarityPct
              ngramContainmentPct
              tokenOverlapPct
              semanticSimilarityPct
              longestCommonRunWords
              longestCommonRunText
              baselineDeltaPct
              exceedsThreshold
            }
          }
        }
      }
    `;
  let result: SnapshotQueryResult;
  let cachedSchema = false;
  try {
    result = await executeVardaRequest('new_gospel', query(true));
  } catch (error) {
    if (!(error instanceof Error) || !error.message.includes('Unknown field "wordRunCounts"')) throw error;
    cachedSchema = true;
    result = await executeVardaRequest('new_gospel', query(false));
  }

  const snapshots = result.queryScriptureComparisonSnapshot ?? [];
  if (snapshots.length === 0) {
    return null;
  }

  const latest = [...snapshots].sort((left, right) =>
    (right.generatedAt ?? '').localeCompare(left.generatedAt ?? ''),
  )[0];

  // An already-running server may still have the previous schema in memory.
  if (cachedSchema) {
    const counts = new Map<string, number[]>();
    for (const verse of latest.verses ?? []) {
      for (const metric of verse.metrics ?? []) {
        const length = metric.longestCommonRunWords;
        if (length == null) continue;
        const key = JSON.stringify([metric.leftTranslationCode, metric.rightTranslationCode]);
        const bands = counts.get(key) ?? Array<number>(7).fill(0);
        const upperBounds = [4, 7, 10, 15, 20, 30, Infinity];
        bands[upperBounds.findIndex(bound => length < bound)]++;
        counts.set(key, bands);
      }
    }
    for (const pair of latest.pairSummaries ?? []) {
      const bands = counts.get(JSON.stringify([pair.leftTranslationCode, pair.rightTranslationCode]));
      if (bands && bands.reduce((sum, n) => sum + n, 0) === pair.verseCount) pair.wordRunCounts = bands;
    }
  }

  return {
    generatedAt: latest.generatedAt ?? '',
    books: latest.books ?? [],
    translationCodes: latest.translationCodes ?? [],
    verbatimRunFlagWords: latest.verbatimRunFlagWords ?? 7,
    thresholdPct: latest.thresholdPct ?? 0,
    totalCopyrightedVerses: latest.totalCopyrightedVerses ?? 0,
    matchedLocalVerses: latest.matchedLocalVerses ?? 0,
    unmatchedCopyrightedVerses: latest.unmatchedCopyrightedVerses ?? 0,
    parseErrorCount: latest.parseErrorCount ?? 0,
    flaggedVerseCount: (latest.verses ?? []).filter(
      (verse) => verse.hasLocalMatch && (verse.metrics ?? []).some((metric) => metric.comparisonKind === 'local_vs_copyright' && metric.exceedsThreshold),
    ).length,
    pairSummaries: latest.pairSummaries ?? [],
    histogramBuckets: latest.histogramBuckets ?? [],
    verses: latest.verses ?? [],
  };
}

function formatPct(value: number | null | undefined): string {
  if (typeof value !== 'number' || Number.isNaN(value)) {
    return '0%';
  }
  return `${Math.round(value)}%`;
}

function scoreTone(value: number, threshold: number): string {
  if (value >= threshold) {
    return '#743636';
  }
  if (value >= Math.max(50, threshold - 10)) {
    return '#245f39';
  }
  return '#363636';
}


function buildBaselineOutliers(
  verses: ComparisonVerseRecord[],
  leftCode: string,
  rightCode: string,
): ComparisonVerseRecord[] {
  return verses
    .filter((verse) => !verse.hasLocalMatch)
    .filter((verse) =>
      (verse.metrics ?? []).some(
        (metric) =>
          metric.comparisonKind === 'baseline'
          && metric.leftTranslationCode === leftCode
          && metric.rightTranslationCode === rightCode,
      ),
    )
    .sort((left, right) => {
      const leftScore = (left.metrics ?? []).find(
        (metric) =>
          metric.comparisonKind === 'baseline'
          && metric.leftTranslationCode === leftCode
          && metric.rightTranslationCode === rightCode,
      )?.sequenceSimilarityPct ?? 0;
      const rightScore = (right.metrics ?? []).find(
        (metric) =>
          metric.comparisonKind === 'baseline'
          && metric.leftTranslationCode === leftCode
          && metric.rightTranslationCode === rightCode,
      )?.sequenceSimilarityPct ?? 0;
      return rightScore - leftScore;
    });
}

const thresholds = [
  { label: '10 or more', from: 3 },
  { label: '15 or more', from: 4 },
  { label: '20 or more', from: 5 },
  { label: '30 or more', from: 6 },
];

function WordMatrixes({ snapshot }: { snapshot: SnapshotViewModel }) {
  const [tab, setTab] = useState<string | null>('20 or more');
  const selected = thresholds.find(t => t.label === tab) ?? thresholds[2];
  const codes = snapshot.translationCodes.filter(code => code !== 'mark-m-mattison');
  const summaries = snapshot.pairSummaries;
  if (!summaries.length || summaries.some(pair => pair.wordRunCounts?.length !== 7)) {
    return <Text>Run the comparison again to generate word matrices</Text>;
  }

  return (
    <Stack gap="lg">
      <Title order={3}>Verses sharing {selected.label} identical words in a row</Title>
      <Tabs value={selected.label} onChange={setTab}>
        <Tabs.List aria-label="Identical words in a row">
          {thresholds.map(t => <Tabs.Tab key={t.label} value={t.label}>{t.label} words</Tabs.Tab>)}
        </Tabs.List>
        <Tabs.Panel value={selected.label} pt="sm">
          <Text mb="sm">
            Each number is how many verses have a run of {selected.label} identical words, in the same order,
            between the two versions. Darker means more verses.
          </Text>
          <WordMatrix codes={codes} summaries={summaries} from={selected.from} />
        </Tabs.Panel>
      </Tabs>
    </Stack>
  );
}

function WordMatrix({ codes, summaries, from }: {
  codes: string[];
  summaries: PairSummaryRecord[];
  from: number;
}) {
  function countForBand(pair?: PairSummaryRecord) {
    const counts = pair?.wordRunCounts;
    return counts && counts.slice(from).reduce((sum, n) => sum + n, 0);
  }
  const maximum = Math.max(1, ...summaries.filter(pair =>
    codes.includes(pair.rightTranslationCode ?? '') &&
    (pair.leftTranslationCode === 'LOCAL' || codes.includes(pair.leftTranslationCode ?? ''))
  ).map(pair => countForBand(pair) ?? 0));
  const palette = ['#efe3da', '#ead1c9', '#ce9188', '#a05d5b', '#743538'];

  return (
    <ScrollArea>
      <Table withTableBorder withColumnBorders>
        <Table.Thead><Table.Tr>
          <Table.Th>Compared with</Table.Th>
          {codes.map(code => <Table.Th key={code}>{code}</Table.Th>)}
        </Table.Tr></Table.Thead>
        <Table.Tbody>
          {['LOCAL', ...codes].map(left => {
            const pairs = codes.map(right => summaries.find(pair =>
              (pair.leftTranslationCode === left && pair.rightTranslationCode === right)
              || (pair.leftTranslationCode === right && pair.rightTranslationCode === left)
            ));
            const totals = new Set(pairs.flatMap(pair => pair?.verseCount == null ? [] : [pair.verseCount]));
            return <Table.Tr key={left}>
              <Table.Th scope="row">
                {left === 'LOCAL' ? 'Unified Gospel' : left}
                {totals.size === 1 && <Text size="xs">{[...totals][0].toLocaleString()} comparisons per pair</Text>}
              </Table.Th>
              {codes.map((right, col) => {
                const pair = pairs[col];
                const count = countForBand(pair);
                const shade = Math.min(4, Math.floor(5 * Math.sqrt((count ?? 0) / maximum)));
                return <Table.Td key={right}>
                  {left === right ? '—' : count == null ? 'Unavailable' : (
                    <div style={{ backgroundColor: palette[shade], color: shade >= 3 ? '#fff' : '#27231f',
                      borderRadius: 8, padding: '7px 10px', textAlign: 'center', fontWeight: 600 }}>
                      {count.toLocaleString()}
                    </div>
                  )}
                  {totals.size > 1 && pair?.verseCount != null && <Text size="xs">of {pair.verseCount.toLocaleString()}</Text>}
                </Table.Td>;
              })}
            </Table.Tr>;
          })}
        </Table.Tbody>
      </Table>
    </ScrollArea>
  );
}

export function ComparisonPage() {
  const [snapshot, setSnapshot] = useState<SnapshotViewModel | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedPair, setSelectedPair] = useState<{ left: string; right: string } | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        setLoading(true);
        setError(null);
        const nextSnapshot = await fetchSnapshot();
        if (!cancelled) {
          setSnapshot(nextSnapshot);
        }
      } catch (loadError) {
        if (!cancelled) {
          console.error(loadError);
          setError(loadError instanceof Error ? loadError.message : 'Failed to load comparison snapshot.');
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    void load();

    return () => {
      cancelled = true;
    };
  }, []);

  const localSummaries = useMemo(
    () => (snapshot?.pairSummaries ?? []).filter((summary) => summary.comparisonKind === 'local_vs_copyright'),
    [snapshot],
  );
  const baselineSummaries = useMemo(
    () => (snapshot?.pairSummaries ?? []).filter((summary) => summary.comparisonKind === 'baseline'),
    [snapshot],
  );
  const baselineOutliers = useMemo(() => {
    if (!snapshot || !selectedPair) {
      return [];
    }
    return buildBaselineOutliers(snapshot.verses, selectedPair.left, selectedPair.right).slice(0, 25);
  }, [selectedPair, snapshot]);
  const [flaggedPage, setFlaggedPage] = useState(0);
  const PAGE_SIZE = 50;

  const localVerses = useMemo(
    () => (snapshot?.verses ?? []).filter((verse) =>
      verse.hasLocalMatch
      && (verse.metrics ?? []).some(
        (metric) =>
          metric.comparisonKind === 'local_vs_copyright'
          && metric.exceedsThreshold
          && (metric.ngramContainmentPct ?? 0) >= 49,
      ),
    ),
    [snapshot],
  );

  const flaggedPageVerses = useMemo(
    () => localVerses.slice(flaggedPage * PAGE_SIZE, (flaggedPage + 1) * PAGE_SIZE),
    [localVerses, flaggedPage],
  );
  const flaggedPageCount = Math.ceil(localVerses.length / PAGE_SIZE);

  return (
    <Container
      size="100%"
      px="lg"
      py="md"
      style={{
        flex: 1,
        minHeight: 0,
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
        background:
          'radial-gradient(circle at top left, rgba(191, 164, 92, 0.10), transparent 22%), linear-gradient(180deg, #faf7f0 0%, #f3efe5 100%)',
        color: '#1f1b16',
        borderRadius: 18,
        border: '1px solid #e7dfd1',
      }}
    >
      <Drawer
        opened={selectedPair !== null}
        onClose={() => setSelectedPair(null)}
        title={selectedPair ? `${selectedPair.left} vs ${selectedPair.right}` : 'Baseline details'}
        position="right"
        size="lg"
      >
        <Stack gap="sm">
          {baselineOutliers.map((verse) => {
            const metric = (verse.metrics ?? []).find(
              (entry) =>
                entry.comparisonKind === 'baseline'
                && entry.leftTranslationCode === selectedPair?.left
                && entry.rightTranslationCode === selectedPair?.right,
            );
            return (
              <Card key={verse.id ?? verse.reference} withBorder radius="md">
                <Group justify="space-between" align="flex-start">
                  <Text fw={700}>{verse.reference}</Text>
                  <Badge color={metric?.sequenceSimilarityPct && metric.sequenceSimilarityPct >= (snapshot?.thresholdPct ?? 0) ? 'red' : 'green'}>
                    {formatPct(metric?.sequenceSimilarityPct)}
                  </Badge>
                </Group>
                <Text size="sm" c="dimmed">
                  Token overlap {formatPct(metric?.tokenOverlapPct)} | Semantic {formatPct(metric?.semanticSimilarityPct)}
                </Text>
                {(verse.texts ?? []).map((text) => (
                  <Text key={`${verse.id}-${text.translationCode}`} size="sm">
                    <Text span fw={700}>{text.translationCode}:</Text> {stripCanonicalDisplayMarkers(text.text)}
                  </Text>
                ))}
              </Card>
            );
          })}
          {baselineOutliers.length === 0 ? <Text size="sm">No verse details were found for this pair.</Text> : null}
        </Stack>
      </Drawer>

      <Stack gap="md" style={{ flex: 1, minHeight: 0 }}>
        <div>
          <Group justify="space-between" align="flex-start">
            <div>
              <Title order={2} c="#1f1b16">Scripture Comparison</Title>
              <Text c="#6b6257" size="sm">
                Read-only comparison snapshot from the local database.
              </Text>
            </div>
            {snapshot ? (
              <Badge size="lg" variant="light" color="gray">
                {snapshot.generatedAt ? new Date(snapshot.generatedAt).toLocaleString() : 'No timestamp'}
              </Badge>
            ) : null}
          </Group>
        </div>

        {error ? (
          <Alert color="red" title="Error">
            {error}
          </Alert>
        ) : null}

        {loading ? (
          <Group justify="center" py="xl">
            <Loader color="yellow" />
          </Group>
        ) : null}

        {!loading && !error && !snapshot ? (
          <Alert color="blue" title="No snapshot">
            Run `cargo run --bin compare -- ...` to generate the comparison snapshot. If the app was already open when the CLI finished, reload or restart the app so it re-reads the updated local `new-gospel` data.
          </Alert>
        ) : null}

        {!loading && snapshot ? (
          <>
            <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md">
              <Paper radius="lg" p="md" style={{ background: '#fffdfa', border: '1px solid #e7dfd1' }}>
                <Text size="xs" c="#7a7165" tt="uppercase">Verses flagged</Text>
                <Title order={3} c="#1f1b16">{snapshot.flaggedVerseCount}</Title>
                <Text size="xs" c="#7a7165">Local verses where any translation exceeds threshold</Text>
              </Paper>
              <Paper radius="lg" p="md" style={{ background: '#fffdfa', border: '1px solid #e7dfd1' }}>
                <Text size="xs" c="#7a7165" tt="uppercase">Matched / Unmatched</Text>
                <Title order={3} c="#1f1b16">{snapshot.matchedLocalVerses} / {snapshot.unmatchedCopyrightedVerses}</Title>
              </Paper>
            </SimpleGrid>

            <Tabs defaultValue="baseline" style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
              <Tabs.List grow>
                <Tabs.Tab value="baseline">Baseline</Tabs.Tab>
                <Tabs.Tab value="flagged">Flagged Verses</Tabs.Tab>
                <Tabs.Tab value="overview">Overview</Tabs.Tab>
                <Tabs.Tab value="word-matrixes">Word Matrixes</Tabs.Tab>
              </Tabs.List>

              <Tabs.Panel value="word-matrixes" pt="md" style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}>
                <WordMatrixes snapshot={snapshot} />
              </Tabs.Panel>
              <Tabs.Panel value="baseline" pt="md" style={{ flex: 1, minHeight: 0 }}>
                <Stack gap="md">
                  <Text size="sm">Each percentage shows how much exact wording two versions share: the average share of four-word phrases that appear, in the same order, in both versions of a verse.</Text>
                  <ScrollArea>
                    <Table withTableBorder withColumnBorders>
                      <Table.Thead>
                        <Table.Tr>
                          <Table.Th>vs</Table.Th>
                          {snapshot.translationCodes
                            .filter((code) => code !== 'mark-m-mattison')
                            .map((code) => (
                              <Table.Th key={code}>{code}</Table.Th>
                            ))}
                        </Table.Tr>
                      </Table.Thead>
                      <Table.Tbody>
                        <Table.Tr>
                          <Table.Td fw={600}>Unified Gospel</Table.Td>
                          {snapshot.translationCodes
                            .filter((code) => code !== 'mark-m-mattison')
                            .map((columnCode) => {
                              const summary = localSummaries.find(
                                (entry) =>
                                  (entry.leftTranslationCode === 'LOCAL' && entry.rightTranslationCode === columnCode)
                                  || (entry.leftTranslationCode === columnCode && entry.rightTranslationCode === 'LOCAL'),
                              );
                              if (!summary) {
                                return <Table.Td key={columnCode} c="dimmed">–</Table.Td>;
                              }
                              const score = summary.averageNgramContainmentPct ?? 0;
                              return (
                                <Table.Td key={columnCode}>
                                  <div
                                    style={{
                                      background: scoreTone(score, snapshot.thresholdPct),
                                      color: '#f7f3ea',
                                      borderRadius: 10,
                                      padding: '8px 12px',
                                      textAlign: 'center',
                                      fontWeight: 600,
                                    }}
                                  >
                                    {formatPct(score)}
                                  </div>
                                </Table.Td>
                              );
                            })}
                        </Table.Tr>
                        {/* Copyrighted vs copyrighted baseline rows */}
                        {snapshot.translationCodes
                          .filter((code) => code !== 'mark-m-mattison')
                          .map((rowCode) => (
                            <Table.Tr key={rowCode}>
                              <Table.Td fw={500}>{rowCode}</Table.Td>
                              {snapshot.translationCodes
                                .filter((code) => code !== 'mark-m-mattison')
                                .map((columnCode) => {
                                  if (rowCode === columnCode) {
                                    return <Table.Td key={columnCode} c="dimmed">-</Table.Td>;
                                  }
                                  const summary = baselineSummaries.find(
                                    (entry) =>
                                      (entry.leftTranslationCode === rowCode && entry.rightTranslationCode === columnCode)
                                      || (entry.leftTranslationCode === columnCode && entry.rightTranslationCode === rowCode),
                                  );
                                  if (!summary) {
                                    return <Table.Td key={columnCode} c="dimmed">-</Table.Td>;
                                  }
                                  // const score = summary.averageSequenceSimilarityPct ?? 0;
                                  const score = summary.averageNgramContainmentPct ?? 0;
                                  return (
                                    <Table.Td key={columnCode}>
                                      <button
                                        type="button"
                                        onClick={() => setSelectedPair({
                                          left: summary.leftTranslationCode ?? rowCode,
                                          right: summary.rightTranslationCode ?? columnCode,
                                        })}
                                        style={{
                                          width: '100%',
                                          background: scoreTone(score, snapshot.thresholdPct),
                                          color: '#f7f3ea',
                                          border: 'none',
                                          borderRadius: 10,
                                          padding: '8px 12px',
                                          cursor: 'pointer',
                                          fontWeight: 600,
                                        }}
                                      >
                                        {formatPct(score)}
                                      </button>
                                    </Table.Td>
                                  );
                                })}
                            </Table.Tr>
                          ))}
                      </Table.Tbody>
                    </Table>
                  </ScrollArea>
                  <Group gap="md">
                    <Badge variant="filled" style={{ background: '#743636' }}>{`>= ${formatPct(snapshot.thresholdPct)} high similarity`}</Badge>
                    <Badge variant="filled" style={{ background: '#245f39' }}>convergence zone</Badge>
                    <Badge variant="filled" style={{ background: '#363636' }}>below threshold</Badge>
                  </Group>
                </Stack>
              </Tabs.Panel>

              <Tabs.Panel value="flagged" pt="md" style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
                <Stack gap="sm" style={{ flex: 1, minHeight: 0 }}>
                  <Group justify="space-between">
                    <Text size="sm" c="#6b6257">{localVerses.length} verses — showing {flaggedPage * PAGE_SIZE + 1}–{Math.min((flaggedPage + 1) * PAGE_SIZE, localVerses.length)}</Text>
                    <Group gap="xs">
                      <button
                        type="button"
                        disabled={flaggedPage === 0}
                        onClick={() => setFlaggedPage((p) => p - 1)}
                        style={{ padding: '4px 12px', borderRadius: 8, border: '1px solid #e7dfd1', background: '#fffdfa', cursor: flaggedPage === 0 ? 'default' : 'pointer', opacity: flaggedPage === 0 ? 0.4 : 1 }}
                      >
                        ← Prev
                      </button>
                      <Text size="sm">{flaggedPage + 1} / {flaggedPageCount}</Text>
                      <button
                        type="button"
                        disabled={flaggedPage >= flaggedPageCount - 1}
                        onClick={() => setFlaggedPage((p) => p + 1)}
                        style={{ padding: '4px 12px', borderRadius: 8, border: '1px solid #e7dfd1', background: '#fffdfa', cursor: flaggedPage >= flaggedPageCount - 1 ? 'default' : 'pointer', opacity: flaggedPage >= flaggedPageCount - 1 ? 0.4 : 1 }}
                      >
                        Next →
                      </button>
                    </Group>
                  </Group>
                  <ScrollArea style={{ flex: 1 }}>
                    <Stack gap="sm">
                      {flaggedPageVerses.map((verse) => {
                        const localText = (verse.texts ?? []).find((text) => text.sourceKind === 'local')?.text ?? '';
                        const verseMetrics = (verse.metrics ?? [])
                          .filter((metric) => metric.comparisonKind === 'local_vs_copyright')
                          .sort((left, right) => (right.ngramContainmentPct ?? 0) - (left.ngramContainmentPct ?? 0));

                        return (
                          <Card
                            key={verse.id ?? `${verse.reference}-${verse.newGospelVerseUid}-${verse.matchOrdinal}`}
                            radius="lg"
                            p="md"
                            style={{ background: '#fffdfa', border: '1px solid #e7dfd1' }}
                          >
                            <Stack gap="xs">
                              <Group justify="space-between" align="flex-start">
                                <div>
                                  <Text fw={700}>{verse.reference}</Text>
                                  <Text size="sm" c="#6b6257">
                                    Local verse {verse.newGospelVerseNumber} | {verse.supportingReferenceDisplay}
                                  </Text>
                                </div>
                                <Group gap="xs">
                                  {verseMetrics.map((metric) => (
                                    <Badge
                                      key={`${verse.id}-${metric.rightTranslationCode}`}
                                      variant="filled"
                                      style={{
                                        background: scoreTone(metric.ngramContainmentPct ?? 0, snapshot.thresholdPct),
                                        color: '#f7f3ea',
                                      }}
                                    >
                                      {(metric.rightTranslationCode ?? '?')} {formatPct(metric.ngramContainmentPct)}
                                    </Badge>
                                  ))}
                                </Group>
                              </Group>
                              <Text size="sm">{stripCanonicalDisplayMarkers(localText)}</Text>
                              <SimpleGrid cols={{ base: 1, md: 2 }} spacing="sm">
                                {verseMetrics.map((metric) => {
                                  const copyrightText = (verse.texts ?? []).find((t) => t.translationCode === metric.rightTranslationCode)?.text ?? '';
                                  return (
                                    <Paper key={`${verse.id}-${metric.rightTranslationCode}-detail`} p="sm" radius="md" style={{ background: '#f7f2e8', border: '1px solid #e7dfd1' }}>
                                      <Text fw={700} mb={4}>{metric.rightTranslationCode}</Text>
                                      {copyrightText ? <Text size="sm" mb={4}>{stripCanonicalDisplayMarkers(copyrightText)}</Text> : null}
                                      <Text size="sm" c="#6b6257">Run {metric.longestCommonRunWords ?? 0} words | &ldquo;{metric.longestCommonRunText ?? ''}&rdquo;</Text>
                                    </Paper>
                                  );
                                })}
                              </SimpleGrid>
                            </Stack>
                          </Card>
                        );
                      })}
                      {localVerses.length === 0 ? (
                        <Text size="sm" c="#6b6257">No flagged verses at 49%+ ngram containment.</Text>
                      ) : null}
                    </Stack>
                  </ScrollArea>
                </Stack>
              </Tabs.Panel>

              <Tabs.Panel value="overview" pt="md" style={{ flex: 1, minHeight: 0 }}>
                <Stack gap="md">
                  <Card radius="lg" p="md" style={{ background: '#fffdfa', border: '1px solid #e7dfd1' }}>
                    <Text fw={700} mb="sm">Distribution histogram</Text>
                    <Stack gap="xs">
                      {snapshot.histogramBuckets.map((bucket) => {
                        const maxCount = Math.max(...snapshot.histogramBuckets.map((item) => item.count ?? 0), 1);
                        const width = `${(((bucket.count ?? 0) / maxCount) * 100).toFixed(1)}%`;
                        return (
                          <div key={`${bucket.bucketStart}-${bucket.bucketEnd}`}>
                            <Group justify="space-between" mb={4}>
                              <Text size="sm">{bucket.bucketStart}-{bucket.bucketEnd}%</Text>
                              <Text size="sm">{bucket.count ?? 0}</Text>
                            </Group>
                            <div style={{ height: 10, background: 'rgba(255,255,255,0.08)', borderRadius: 999 }}>
                              <div
                                style={{
                                  width,
                                  height: '100%',
                                  borderRadius: 999,
                                  background: 'linear-gradient(90deg, #7f6a2a 0%, #c59a36 100%)',
                                }}
                              />
                            </div>
                          </div>
                        );
                      })}
                    </Stack>
                  </Card>
                </Stack>
              </Tabs.Panel>
            </Tabs>
          </>
        ) : null}
      </Stack>
    </Container>
  );
}