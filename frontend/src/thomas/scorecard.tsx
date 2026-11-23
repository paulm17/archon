import { useEffect, useState } from 'react';
import {
  Alert,
  Badge,
  Card,
  Grid,
  Group,
  Loader,
  Stack,
  Table,
  Text,
  Title,
} from '@mantine/core';
import { executeVardaRequest } from '../lib/client';

type ScoreRecord = {
  id: string;
  rating: string;
  note: string | null;
  criterion: {
    id: string;
    description: string;
  };
};

type LogionScoreRecord = {
  id: string;
  total: number | null;
  outOf: number | null;
  isClosest: boolean | null;
  footnote: string | null;
  translator: {
    id: string;
    name: string;
    shortName: string | null;
  };
  scores: ScoreRecord[];
};

type CriterionRecord = {
  id: string;
  description: string;
  copticToken: string | null;
  failureMode: string | null;
  weight: number | null;
};

type LogionScoresQueryResult = {
  getLogion?: {
    id?: string | null;
    number?: string | null;
    criteria?: Array<{
      id?: string | null;
      description?: string | null;
      copticToken?: string | null;
      failureMode?: string | null;
      weight?: number | null;
    } | null> | null;
    scores?: Array<{
      id?: string | null;
      total?: number | null;
      outOf?: number | null;
      isClosest?: boolean | null;
      footnote?: string | null;
      translator?: {
        id?: string | null;
        name?: string | null;
        shortName?: string | null;
      } | null;
      scores?: Array<{
        id?: string | null;
        rating?: string | null;
        note?: string | null;
        criterion?: {
          id?: string | null;
          description?: string | null;
        } | null;
      } | null> | null;
    } | null> | null;
  } | null;
};

type ScorecardData = {
  logionNumber: string;
  criteria: CriterionRecord[];
  outOf: number;
  translators: {
    shortName: string;
    name: string;
    total: number;
    outOf: number;
    isClosest: boolean;
    footnote: string | null;
    scoreMap: Map<string, { rating: string; note: string | null }>;
  }[];
  footnotes: string[];
};

function normalizeScorecardData(result: LogionScoresQueryResult): ScorecardData | null {
  const logion = result.getLogion;

  if (!logion?.id || !logion?.number) {
    console.log('[Scorecard] FAIL: logion missing id or number');
    return null;
  }

  const criteria: CriterionRecord[] = (logion.criteria ?? [])
    .flatMap((c) => {
      if (!c?.id || !c?.description) return [];
      return [{ id: c.id, description: c.description, copticToken: c.copticToken ?? null, failureMode: c.failureMode ?? null, weight: c.weight ?? null }];
    });

  const outOf = criteria.length > 0 ? criteria.length : 7;

  const translatorScores = (logion.scores ?? [])
    .flatMap((ls) => {
      if (!ls?.id || !ls?.translator?.id || !ls?.translator?.name) {
        return [];
      }

      const scoreMap = new Map<string, { rating: string; note: string | null }>();
      for (const score of ls.scores ?? []) {
        if (!score?.criterion?.id) continue;
        scoreMap.set(score.criterion.id, {
          rating: score.rating ?? '',
          note: score.note ?? null,
        });
      }

      return [{
        id: ls.id,
        shortName: ls.translator.shortName ?? ls.translator.name,
        name: ls.translator.name,
        total: ls.total ?? 0,
        outOf: ls.outOf ?? outOf,
        isClosest: ls.isClosest ?? false,
        footnote: ls.footnote ?? null,
        scoreMap,
      }];
    })
    .sort((a, b) => a.shortName.localeCompare(b.shortName));

  const footnotes = translatorScores
    .map((t) => t.footnote)
    .filter((f): f is string => f !== null && f.trim().length > 0);

  return {
    logionNumber: logion.number,
    criteria,
    outOf,
    translators: translatorScores,
    footnotes: [...new Set(footnotes)],
  };
}

async function fetchScorecardData(uid: string): Promise<ScorecardData | null> {
  const result = await executeVardaRequest<LogionScoresQueryResult>(
    'thomas',
    `
      query GetLogionScores($uid: ID!) {
        getLogion(uid: $uid) {
          id
          number
          criteria {
            id
            description
            copticToken
            failureMode
            weight
          }
          scores {
            id
            total
            outOf
            isClosest
            footnote
            translator {
              id
              name
              shortName
            }
            scores {
              id
              rating
              note
              criterion {
                id
                description
              }
            }
          }
        }
      }
    `,
    { uid },
  );

  const normalized = normalizeScorecardData(result);
  return normalized;
}

function getRatingDisplay(rating: string): { symbol: string; color: string } {
  const normalized = rating.trim().toLowerCase();
  if (normalized === 'accurate' || normalized === '✓') {
    return { symbol: '✓', color: 'green' };
  }
  if (normalized === 'partial' || normalized === '~') {
    return { symbol: '~', color: 'blue' };
  }
  if (normalized === 'inaccurate' || normalized === 'omitted' || normalized === '✕') {
    return { symbol: '✕', color: 'red' };
  }
  return { symbol: rating, color: 'gray' };
}

function RatingCell({ rating, note }: { rating: string; note: string | null }) {
  const { symbol, color } = getRatingDisplay(rating);

  return (
    <Text>
      <Text span c={color} fw={500}>
        {symbol}
      </Text>
      {note ? (
        <Text span c={color}>
          {' '}{note}
        </Text>
      ) : null}
    </Text>
  );
}

export function Scorecard({ logionUid }: { logionUid: string | null }) {
  const [data, setData] = useState<ScorecardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!logionUid) {
      setLoading(false);
      return;
    }

    let cancelled = false;

    async function load() {
      try {
        setLoading(true);
        setError(null);

        const loaded = await fetchScorecardData(logionUid);
        if (cancelled) return;

        setData(loaded);
      } catch (loadError) {
        if (!cancelled) {
          console.error(loadError);
          setError('Failed to load scorecard data.');
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
  }, [logionUid]);

  if (!logionUid) {
    return (
      <Alert color="blue" title="No logion selected">
        Select a logion to view its scorecard.
      </Alert>
    );
  }

  if (loading) {
    return (
      <Group justify="center" py="xl">
        <Loader />
      </Group>
    );
  }

  if (error) {
    return (
      <Alert color="red" title="Error">
        {error}
      </Alert>
    );
  }

  if (!data || data.translators.length === 0) {
    return (
      <Alert color="blue" title="No scorecard data">
        No scorecard data available for this logion.
      </Alert>
    );
  }

  const translatorColumns = data.translators;

  console.log('[Scorecard] TABLE ROWS:', data.criteria.map(criterion => ({
    criterion: {
      id: criterion.id,
      description: criterion.description,
      copticToken: criterion.copticToken,
      failureMode: criterion.failureMode,
    },
    scores: Object.fromEntries(translatorColumns.map(t => {
      const score = t.scoreMap.get(criterion.id);
      return [t.shortName, score ? { rating: score.rating, note: score.note } : null];
    })),
  })));

  const tableRows = data.criteria.map((criterion) => (
    <Table.Tr key={criterion.id}>
      <Table.Td>
        <Text size="sm" fw={500}>
          {criterion.description}
        </Text>
        {criterion.failureMode ? (
          <Text size="xs" c="red">
            failure: {criterion.failureMode}
          </Text>
        ) : null}
      </Table.Td>
      {translatorColumns.map((translator) => {
        const score = translator.scoreMap.get(criterion.id);
        return (
          <Table.Td key={`${translator.shortName}-${criterion.id}`}>
            {score ? (
              <RatingCell rating={score.rating} note={score.note} />
            ) : (
              <Text c="dimmed">—</Text>
            )}
          </Table.Td>
        );
      })}
    </Table.Tr>
  ));

  return (
    <Stack gap="xl">
      <Stack gap="xs">
        <Title order={3}>
          Logion {data.logionNumber} — translation fidelity
        </Title>
        <Text c="dimmed">
          Scored against Coptic morphological analysis ({data.outOf} criteria)
        </Text>
      </Stack>

      <Grid>
        {translatorColumns.map((translator) => (
          <Grid.Col key={translator.shortName} span={{ base: 12, sm: 6, md: 4 }}>
            <Card withBorder padding="lg" radius="md">
              <Stack gap="xs">
                <Text fw={700} size="lg">
                  {translator.shortName}
                </Text>
                <Text fw={700} size="2rem" style={{ lineHeight: 1 }}>
                  {translator.total}
                  <Text span size="sm" c="dimmed" fw={400}>
                    /{translator.outOf}
                  </Text>
                </Text>
                {translator.isClosest ? (
                  <Badge color="blue" variant="light" radius="xl">
                    Closest
                  </Badge>
                ) : null}
              </Stack>
            </Card>
          </Grid.Col>
        ))}
      </Grid>

      <Table striped highlightOnHover withTableBorder>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Criterion</Table.Th>
            {translatorColumns.map((translator) => (
              <Table.Th key={translator.shortName}>{translator.shortName}</Table.Th>
            ))}
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>{tableRows}</Table.Tbody>
      </Table>

      <Stack gap="xs">
        <Text size="sm">
          <Text span c="green" fw={500}>✓ accurate</Text>
          {' · '}
          <Text span c="blue" fw={500}>~ partial</Text>
          {' · '}
          <Text span c="red" fw={500}>✕ inaccurate or omitted</Text>
        </Text>
        {data.footnotes.map((footnote, index) => (
          <Text key={index} size="sm" c="dimmed">
            {footnote}
          </Text>
        ))}
      </Stack>
    </Stack>
  );
}
