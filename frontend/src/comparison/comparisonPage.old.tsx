import { useEffect, useMemo, useState } from 'react';
import {
  Alert,
  Badge,
  Card,
  Container,
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
  comparisonKind?: string | null;
  leftTranslationCode?: string | null;
  rightTranslationCode?: string | null;
  averageNgramContainmentPct?: number | null;
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
  const result = await executeVardaRequest<SnapshotQueryResult>(
    'new_gospel',
    `
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
            comparisonKind
            leftTranslationCode
            rightTranslationCode
            averageNgramContainmentPct
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
    `,
  );

  const snapshots = result.queryScriptureComparisonSnapshot ?? [];
  if (snapshots.length === 0) {
    return null;
  }

  const latest = [...snapshots].sort((left, right) =>
    (right.generatedAt ?? '').localeCompare(left.generatedAt ?? ''),
  )[0];

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


export function ComparisonPage() {
  const [snapshot, setSnapshot] = useState<SnapshotViewModel | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

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
          setError('Failed to load comparison snapshot.');
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

        {!loading && !snapshot ? (
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
              </Tabs.List>

              <Tabs.Panel value="baseline" pt="md" style={{ flex: 1, minHeight: 0 }}>
                <Stack gap="md">
                  <ScrollArea>
                    <Table withTableBorder withColumnBorders>
                      <Table.Thead>
                        <Table.Tr>
                          <Table.Th>Unified Gospel vs</Table.Th>
                          {snapshot.translationCodes
                            .filter((code) => code !== 'mark-m-mattison')
                            .map((code) => (
                              <Table.Th key={code}>{code}</Table.Th>
                            ))}
                        </Table.Tr>
                      </Table.Thead>
                      <Table.Tbody>
                        <Table.Tr>
                          <Table.Td fw={600}>Avg ngram</Table.Td>
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