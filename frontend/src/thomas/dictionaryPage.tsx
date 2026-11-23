import { useEffect, useState } from 'react';
import { useParams } from 'react-router-dom';
import {
  Alert,
  Card,
  Container,
  Divider,
  Group,
  Loader,
  SimpleGrid,
  Stack,
  Table,
  Text,
  Title,
  Badge,
} from '@mantine/core';
import { executeVardaRequest } from '../lib/client';

type TokenMorphology = {
  root?: string | null;
  literal?: string | null;
  grammaticalFn?: string | null;
  partOfSpeech?: string | null;
  person?: string | null;
  number?: string | null;
  gender?: string | null;
  tense?: string | null;
  mood?: string | null;
  voice?: string | null;
  caseVal?: string | null;
};

type LexemeDefinition = {
  id?: string | null;
  source?: string | null;
  dictionaryId?: string | null;
  dictionaryHeadword?: string | null;
  definition?: string | null;
  senses?: (string | null)[] | null;
  outline?: (string | null)[] | null;
};

type LexemeForm = {
  id?: string | null;
  form?: string | null;
  transliteration?: string | null;
  morphology?: string | null;
  count?: number | null;
};

type LexemeTranslationEquivalent = {
  id?: string | null;
  english?: string | null;
  source?: string | null;
  relation?: string | null;
  count?: number | null;
};

type LexemeUsageExample = {
  id?: string | null;
  coptic?: string | null;
  interlinearGloss?: string | null;
  translatorName?: string | null;
  translatorRendering?: string | null;
  logion?: { number?: string | null } | null;
};

type LexicalRendering = {
  id?: string | null;
  english?: string | null;
  normalizedEnglish?: string | null;
  source?: string | null;
  relation?: string | null;
  confidence?: number | null;
};

type LexicalEvidence = {
  id?: string | null;
  targetSpan?: string | null;
  method?: string | null;
  confidence?: number | null;
  translator?: {
    id?: string | null;
    name?: string | null;
    shortName?: string | null;
  } | null;
};

type TranslationAlignment = {
  id?: string | null;
  sourceTokenIndexes?: (number | null)[] | null;
  sourceGloss?: string | null;
  targetSpan?: string | null;
  method?: string | null;
  confidence?: number | null;
  evidenceNotes?: string | null;
  translator?: {
    id?: string | null;
    name?: string | null;
    shortName?: string | null;
  } | null;
  rendering?: LexicalRendering | null;
};

type LexicalSourceGroup = {
  id?: string | null;
  tokenIndexes?: (number | null)[] | null;
  coptic?: string | null;
  interlinearGloss?: string | null;
  groupType?: string | null;
  lexemes?: Array<{
    id?: string | null;
    headword?: string | null;
    transliteration?: string | null;
    partOfSpeech?: string | null;
  } | null> | null;
  tokens?: Array<{
    id?: string | null;
    tokenIndex?: number | null;
    coptic?: string | null;
  } | null> | null;
  renderings?: (LexicalRendering | null)[] | null;
  evidence?: (LexicalEvidence | null)[] | null;
  translationAlignments?: (TranslationAlignment | null)[] | null;
};

type LogionRecord = {
  id?: string | null;
  number?: string | null;
  lexicalSourceGroups?: (LexicalSourceGroup | null)[] | null;
};

type DictionaryMatch = {
  id?: string | null;
  headword?: string | null;
  partOfSpeech?: string | null;
  senses?: (string | null)[] | null;
  frequency?: number | null;
  source?: string | null;
};

type TranslationRange = {
  id?: string | null;
  literal?: (string | null)[] | null;
  acceptable?: (string | null)[] | null;
  notThisUnit?: (string | null)[] | null;
};

type LexemeRecord = {
  id?: string | null;
  headword?: string | null;
  normalizedHeadword?: string | null;
  transliteration?: string | null;
  partOfSpeech?: string | null;
  root?: string | null;
  definitions?: LexemeDefinition[] | null;
  forms?: LexemeForm[] | null;
  translationEquivalents?: LexemeTranslationEquivalent[] | null;
  usageExamples?: LexemeUsageExample[] | null;
};

type TokenLexiconRecord = {
  id?: string | null;
  lookupForms?: (string | null)[] | null;
  warnings?: (string | null)[] | null;
  dictionaryMatches?: DictionaryMatch[] | null;
  translationRange?: TranslationRange | null;
};

type TokenRecord = {
  id?: string | null;
  tokenIndex?: number | null;
  coptic?: string | null;
  greek?: string | null;
  transliteration?: string | null;
  englishGloss?: string | null;
  source?: string | null;
  morphology?: TokenMorphology | null;
  lexeme?: LexemeRecord | null;
  lexicon?: TokenLexiconRecord | null;
  logion?: LogionRecord | null;
};

type DictionaryQueryResult = {
  getToken?: TokenRecord | null;
};

type DictionaryData = {
  token: TokenRecord;
  lexeme: LexemeRecord | null;
  lexicon: TokenLexiconRecord | null;
  sourceGroups: LexicalSourceGroup[];
};

type UsageOutlineRow = {
  level: 'major' | 'minor' | 'source';
  marker: string;
  text: string;
  source: string;
  headword: string;
};

function formatGrammar(morphology: TokenMorphology | null): string | null {
  if (!morphology) return null;
  const parts = [
    morphology.partOfSpeech,
    morphology.person,
    morphology.number,
    morphology.gender,
    morphology.tense,
    morphology.mood,
    morphology.voice,
    morphology.caseVal,
    morphology.grammaticalFn,
    morphology.literal,
  ].filter((p): p is string => typeof p === 'string' && p.trim().length > 0);
  return parts.length > 0 ? parts.join(' ') : null;
}

function compactText(value: string | null | undefined): string | null {
  const compact = value?.replace(/\s+/g, ' ').trim();
  return compact && compact.length > 0 ? compact : null;
}

function uniqueBy<T>(items: T[], keyFor: (item: T) => string): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const item of items) {
    const key = keyFor(item);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(item);
  }
  return out;
}

function nonEmptyStrings(values: (string | null | undefined)[] | null | undefined): string[] {
  return uniqueBy(
    (values ?? []).flatMap((value) => {
      const compact = compactText(value);
      return compact ? [compact] : [];
    }),
    (value) => value.toLocaleLowerCase(),
  );
}

function containingSourceGroups(token: TokenRecord): LexicalSourceGroup[] {
  const tokenId = token.id ?? null;
  const tokenIndex = token.tokenIndex ?? null;
  const groups = token.logion?.lexicalSourceGroups ?? [];
  return groups
    .flatMap((group) => {
      if (!group?.id) return [];
      const containsByToken = tokenId
        ? (group.tokens ?? []).some((groupToken) => groupToken?.id === tokenId)
        : false;
      const containsByIndex = typeof tokenIndex === 'number'
        ? (group.tokenIndexes ?? []).some((idx) => idx === tokenIndex)
        : false;
      return containsByToken || containsByIndex ? [group] : [];
    })
    .sort((a, b) => {
      const aLen = a.tokenIndexes?.length ?? 0;
      const bLen = b.tokenIndexes?.length ?? 0;
      if (bLen !== aLen) return bLen - aLen;
      return (a.tokenIndexes?.[0] ?? 0) - (b.tokenIndexes?.[0] ?? 0);
    });
}

function sourceGroupLabel(group: LexicalSourceGroup): string {
  const indexes = (group.tokenIndexes ?? []).filter((idx): idx is number => typeof idx === 'number');
  return indexes.length > 0 ? indexes.join(', ') : '—';
}

function renderingKey(rendering: LexicalRendering | null | undefined): string {
  return [
    compactText(rendering?.english)?.toLocaleLowerCase() ?? '',
    compactText(rendering?.source)?.toLocaleLowerCase() ?? '',
    compactText(rendering?.relation)?.toLocaleLowerCase() ?? '',
  ].join('|');
}

function sourceGroupRenderings(group: LexicalSourceGroup): LexicalRendering[] {
  return uniqueBy(
    (group.renderings ?? []).flatMap((rendering) => {
      if (!rendering || !compactText(rendering.english)) return [];
      return [rendering];
    }),
    renderingKey,
  ).sort((a, b) => (b.confidence ?? 0) - (a.confidence ?? 0));
}

function sourceGroupEvidence(group: LexicalSourceGroup): TranslationAlignment[] {
  return uniqueBy(
    (group.translationAlignments ?? []).flatMap((alignment) => {
      if (!alignment?.id || !compactText(alignment.targetSpan)) return [];
      return [alignment];
    }),
    (alignment) => [
      alignment.translator?.shortName ?? alignment.translator?.name ?? '',
      compactText(alignment.targetSpan)?.toLocaleLowerCase() ?? '',
      compactText(alignment.method)?.toLocaleLowerCase() ?? '',
      alignment.rendering?.id ?? '',
    ].join('|'),
  ).sort((a, b) => {
    const translatorA = a.translator?.shortName ?? a.translator?.name ?? '';
    const translatorB = b.translator?.shortName ?? b.translator?.name ?? '';
    return translatorA.localeCompare(translatorB);
  });
}

function confidenceLabel(value: number | null | undefined): string {
  return typeof value === 'number' ? value.toFixed(2) : '—';
}

function hasEnglishText(value: string): boolean {
  return /[A-Za-z]/.test(value);
}

function splitSenseText(value: string): string[] {
  return value
    .split(/;|\n/)
    .map((part) => part.trim())
    .filter((part) => part.length > 0 && hasEnglishText(part));
}

function definitionPriority(definition: LexemeDefinition): number {
  const source = definition.source?.toLocaleLowerCase() ?? '';
  if (source.includes('comprehensive coptic lexicon')) return 0;
  if (source.includes('coptic_lexicon_index')) return 1;
  if (source.includes('kellia')) return 2;
  return 3;
}

function definitionEnglishSenses(definition: LexemeDefinition): string[] {
  return uniqueBy(
    [
      ...nonEmptyStrings(definition.senses).flatMap(splitSenseText),
      ...splitSenseText(definition.definition ?? ''),
    ],
    (sense) => sense.toLocaleLowerCase(),
  );
}

function definitionOutlineRows(definitions: LexemeDefinition[] | null | undefined): UsageOutlineRow[] {
  const rows: UsageOutlineRow[] = [];
  const sorted = (definitions ?? [])
    .slice()
    .sort((a, b) => definitionPriority(a) - definitionPriority(b));

  for (const definition of sorted) {
    const source = definition.source ?? '';
    const headword = definition.dictionaryHeadword ?? '';
    const outline = nonEmptyStrings(definition.outline);
    if (outline.length === 0) {
      for (const sense of definitionEnglishSenses(definition)) {
        rows.push({
          level: 'major',
          marker: romanNumeral(rows.filter((row) => row.level === 'major').length + 1),
          text: sense.startsWith('to ') ? sense : `to ${sense}`,
          source,
          headword,
        });
      }
      continue;
    }

    let minor = 0;
    for (const raw of outline) {
      const [kind, ...rest] = raw.split('|');
      const text = compactText(rest.length > 0 ? rest.join('|') : raw);
      if (!text || !hasEnglishText(text)) continue;
      if (kind === 'I') {
        minor = 0;
        rows.push({
          level: 'major',
          marker: romanNumeral(rows.filter((row) => row.level === 'major').length + 1),
          text,
          source,
          headword,
        });
      } else if (kind === '1') {
        minor += 1;
        rows.push({
          level: 'minor',
          marker: `${minor}.`,
          text,
          source,
          headword,
        });
      } else if (kind === 'source') {
        rows.push({
          level: 'source',
          marker: '',
          text,
          source,
          headword,
        });
      } else {
        minor += 1;
        rows.push({
          level: 'minor',
          marker: `${minor}.`,
          text,
          source,
          headword,
        });
      }
    }
  }

  return uniqueBy(rows, (row) => [
    row.level,
    row.text.toLocaleLowerCase(),
    row.source,
    row.headword,
  ].join('|'));
}

function romanNumeral(value: number): string {
  const numerals = ['I', 'II', 'III', 'IV', 'V', 'VI', 'VII', 'VIII', 'IX', 'X'];
  return `${numerals[value - 1] ?? value}.`;
}

function sourceLabel(value: string | null | undefined): string {
  const source = value ?? '';
  if (source.includes('Comprehensive Coptic Lexicon')) return 'CCL';
  if (source.includes('coptic_lexicon_index')) return 'lexicon';
  if (source.includes('coptic_word_index')) return 'word index';
  if (source.includes('translation_range')) return 'translation range';
  if (source.includes('interlinear')) return 'interlinear';
  if (source.includes('wordnet')) return 'WordNet';
  if (source.includes('kellia')) return 'Kellia';
  return source || 'source';
}

async function fetchDictionaryData(tokenId: string): Promise<DictionaryData | null> {
  const result = await executeVardaRequest<DictionaryQueryResult>(
    'thomas',
    `
      query GetTokenDictionary($uid: ID!) {
        getToken(uid: $uid) {
          id
          tokenIndex
          coptic
          greek
          transliteration
          englishGloss
          source
          logion {
            id
            number
            lexicalSourceGroups(first: 300) {
              id
              tokenIndexes
              coptic
              interlinearGloss
              groupType
              tokens(first: 20) {
                id
                tokenIndex
                coptic
              }
              lexemes(first: 20) {
                id
                headword
                transliteration
                partOfSpeech
              }
              renderings(first: 80) {
                id
                english
                normalizedEnglish
                source
                relation
                confidence
              }
              evidence(first: 80) {
                id
                targetSpan
                method
                confidence
                translator {
                  id
                  name
                  shortName
                }
              }
              translationAlignments(first: 120) {
                id
                sourceTokenIndexes
                sourceGloss
                targetSpan
                method
                confidence
                evidenceNotes
                translator {
                  id
                  name
                  shortName
                }
                rendering {
                  id
                  english
                  normalizedEnglish
                  source
                  relation
                  confidence
                }
              }
            }
          }
          morphology {
            root
            literal
            grammaticalFn
            partOfSpeech
            person
            number
            gender
            tense
            mood
            voice
            caseVal
          }
          lexeme {
            id
            headword
            normalizedHeadword
            transliteration
            partOfSpeech
            root
            definitions(first: 50) {
              id
              source
              dictionaryId
              dictionaryHeadword
              definition
              senses
              outline
            }
            forms(first: 50) {
              id
              form
              transliteration
              morphology
              count
            }
            translationEquivalents(first: 50) {
              id
              english
              source
              relation
              count
            }
            usageExamples(first: 50) {
              id
              coptic
              interlinearGloss
              translatorName
              translatorRendering
              logion {
                number
              }
            }
          }
          lexicon {
            id
            lookupForms
            warnings
            dictionaryMatches(first: 50) {
              id
              headword
              partOfSpeech
              senses
              frequency
              source
            }
            translationRange {
              id
              literal
              acceptable
              notThisUnit
            }
          }
        }
      }
    `,
    { uid: tokenId },
  );

  const token = result.getToken;
  if (!token) return null;

  return {
    token,
    lexeme: token.lexeme ?? null,
    lexicon: token.lexicon ?? null,
    sourceGroups: containingSourceGroups(token),
  };
}

export function DictionaryPage() {
  const { tokenId } = useParams<{ tokenId: string }>();
  const [data, setData] = useState<DictionaryData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!tokenId) {
      setLoading(false);
      setError('No token ID provided.');
      return;
    }

    let cancelled = false;

    async function load() {
      try {
        setLoading(true);
        setError(null);

        if (!tokenId) {
          throw new Error('No token ID provided.');
        }

        const loaded = await fetchDictionaryData(tokenId);
        if (cancelled) return;

        if (!loaded) {
          throw new Error('Token not found in database.');
        }

        setData(loaded);
      } catch (loadError) {
        if (!cancelled) {
          console.error(loadError);
          setError('Failed to load dictionary data.');
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
  }, [tokenId]);

  if (loading) {
    return (
      <Container size="100%" py="xl">
        <Group justify="center">
          <Loader />
        </Group>
      </Container>
    );
  }

  if (error) {
    return (
      <Container size="100%" py="md">
        <Alert color="red" title="Error">
          {error}
        </Alert>
      </Container>
    );
  }

  if (!data) {
    return (
      <Container size="100%" py="md">
        <Alert color="blue" title="No data">
          No dictionary data available for this token.
        </Alert>
      </Container>
    );
  }

  const { token, lexeme, lexicon, sourceGroups } = data;
  const grammar = formatGrammar(token.morphology ?? null);
  const hasLexeme = lexeme && lexeme.id;
  const hasLexicon = lexicon && lexicon.id;

  // Group definitions by source
  const definitionsBySource = new Map<string, LexemeDefinition[]>();
  if (lexeme?.definitions) {
    for (const def of uniqueBy(lexeme.definitions, (definition) => [
      definition?.source ?? '',
      definition?.dictionaryHeadword ?? '',
      compactText(definition?.definition)?.toLocaleLowerCase() ?? '',
      nonEmptyStrings(definition?.senses).join('|'),
    ].join('|'))) {
      if (!def?.source || definitionEnglishSenses(def).length === 0) continue;
      const list = definitionsBySource.get(def.source) ?? [];
      list.push(def);
      definitionsBySource.set(def.source, list);
    }
  }

  // Translation equivalents with total count
  const equivalents = uniqueBy(
    (lexeme?.translationEquivalents ?? []).flatMap((eq) => (compactText(eq?.english) ? [eq] : [])),
    (eq) => [
      compactText(eq?.english)?.toLocaleLowerCase() ?? '',
      eq?.source ?? '',
      eq?.relation ?? '',
    ].join('|'),
  );
  const totalEquivalentCount = equivalents.reduce((sum, eq) => sum + (eq?.count ?? 0), 0);
  const usageOutlineRows = definitionOutlineRows(lexeme?.definitions);
  const usageExamples = uniqueBy(
    (lexeme?.usageExamples ?? []).flatMap((example) => (example ? [example] : [])),
    (example) => [
      example.logion?.number ?? '',
      compactText(example.coptic)?.toLocaleLowerCase() ?? '',
      compactText(example.interlinearGloss)?.toLocaleLowerCase() ?? '',
      compactText(example.translatorName)?.toLocaleLowerCase() ?? '',
      compactText(example.translatorRendering)?.toLocaleLowerCase() ?? '',
    ].join('|'),
  );
  const allEvidence = sourceGroups.flatMap((group) =>
    sourceGroupEvidence(group).map((alignment) => ({ group, alignment })),
  );

  return (
    <Container size="100%" py="md" style={{ height: '100%', overflowY: 'auto' }}>
      <Stack gap="lg">
        {/* Header */}
        <Stack gap="xs">
          <Title order={3}>
            Lexicon :: {lexeme?.headword ?? token.coptic ?? 'Unknown'} — {lexeme?.transliteration ?? token.transliteration ?? ''}
          </Title>
          <Text c="dimmed" size="sm">
            {token.greek ? `Greek: ${token.greek}` : null}
            {token.greek && token.englishGloss ? ' · ' : null}
            {token.englishGloss}
          </Text>
        </Stack>

        {/* Large Word */}
        <Card withBorder padding="lg">
          <Text
            ff="serif"
            size="3rem"
            style={{ lineHeight: 1.2 }}
          >
            {token.coptic ?? '—'}
          </Text>
        </Card>

        {/* Basic Info Grid */}
        <SimpleGrid cols={{ base: 1, sm: 2 }}>
          <Card withBorder padding="sm">
            <Text size="xs" c="dimmed" fw={500} tt="uppercase">
              Transliteration
            </Text>
            <Text size="sm">{lexeme?.transliteration ?? token.transliteration ?? '—'}</Text>
          </Card>
          <Card withBorder padding="sm">
            <Text size="xs" c="dimmed" fw={500} tt="uppercase">
              Part of Speech
            </Text>
            <Text size="sm">{lexeme?.partOfSpeech ?? token.morphology?.partOfSpeech ?? '—'}</Text>
          </Card>
          <Card withBorder padding="sm">
            <Text size="xs" c="dimmed" fw={500} tt="uppercase">
              Root Word
            </Text>
            <Text size="sm">{lexeme?.root ?? token.morphology?.root ?? '—'}</Text>
          </Card>
          <Card withBorder padding="sm">
            <Text size="xs" c="dimmed" fw={500} tt="uppercase">
              Grammar
            </Text>
            <Text size="sm">{grammar ?? '—'}</Text>
          </Card>
        </SimpleGrid>

        {/* Forms / Inflections */}
        {lexeme?.forms && lexeme.forms.length > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="sm">
              <Group justify="space-between">
                <Text fw={600} size="sm">
                  Coptic Forms
                </Text>
                <Text size="xs" c="dimmed">
                  {lexeme.forms.length} form(s)
                </Text>
              </Group>
              <Table striped highlightOnHover withTableBorder>
                <Table.Thead>
                  <Table.Tr>
                    <Table.Th>Form</Table.Th>
                    <Table.Th>Transliteration</Table.Th>
                    <Table.Th>Morphology</Table.Th>
                    <Table.Th>Count</Table.Th>
                  </Table.Tr>
                </Table.Thead>
                <Table.Tbody>
                  {lexeme.forms.map((form, i) => (
                    <Table.Tr key={form?.id ?? `${form?.form}-${form?.transliteration}-${form?.morphology}-${form?.count}-${i}`}>
                      <Table.Td>
                        <Text ff="serif" size="sm">
                          {form?.form ?? '—'}
                        </Text>
                      </Table.Td>
                      <Table.Td>{form?.transliteration ?? '—'}</Table.Td>
                      <Table.Td>{form?.morphology ?? '—'}</Table.Td>
                      <Table.Td>{form?.count ?? '—'}</Table.Td>
                    </Table.Tr>
                  ))}
                </Table.Tbody>
              </Table>
            </Stack>
          </Card>
        ) : null}

        {/* Lexical Source Groups */}
        {sourceGroups.length > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="md">
              <Group justify="space-between">
                <Text fw={600} size="sm">
                  Lexical Source Groups — {sourceGroups.length} group(s)
                </Text>
                {token.logion?.number ? (
                  <Text size="xs" c="dimmed">
                    Logion {token.logion.number}
                  </Text>
                ) : null}
              </Group>
              <Stack gap="md">
                {sourceGroups.map((group) => {
                  const renderings = sourceGroupRenderings(group);
                  const evidence = sourceGroupEvidence(group);
                  const lexemeLabels = (group.lexemes ?? [])
                    .flatMap((linkedLexeme) => {
                      const label = compactText(linkedLexeme?.headword);
                      return label ? [label] : [];
                    });

                  return (
                    <Card key={group.id ?? sourceGroupLabel(group)} withBorder padding="sm">
                      <Stack gap="sm">
                        <Group justify="space-between" align="flex-start">
                          <Stack gap={2}>
                            <Group gap="xs">
                              <Text ff="serif" size="lg" fw={500}>
                                {group.coptic ?? '—'}
                              </Text>
                              <Badge variant="light">{group.groupType ?? 'group'}</Badge>
                            </Group>
                            <Text size="sm" c="dimmed">
                              token indexes: {sourceGroupLabel(group)}
                            </Text>
                          </Stack>
                          <Text size="sm" fw={500}>
                            {group.interlinearGloss ?? '—'}
                          </Text>
                        </Group>

                        {lexemeLabels.length > 0 ? (
                          <Group gap="xs">
                            {uniqueBy(lexemeLabels, (label) => label).map((label) => (
                              <Badge key={label} size="sm" variant="outline">
                                {label}
                              </Badge>
                            ))}
                          </Group>
                        ) : null}

                        {renderings.length > 0 ? (
                          <Table.ScrollContainer minWidth={640}>
                            <Table striped highlightOnHover withTableBorder>
                              <Table.Thead>
                                <Table.Tr>
                                  <Table.Th>Rendering</Table.Th>
                                  <Table.Th>Source</Table.Th>
                                  <Table.Th>Relation</Table.Th>
                                  <Table.Th>Confidence</Table.Th>
                                </Table.Tr>
                              </Table.Thead>
                              <Table.Tbody>
                                {renderings.map((rendering) => (
                                  <Table.Tr key={rendering.id ?? renderingKey(rendering)}>
                                    <Table.Td>
                                      <Text fw={500} size="sm">
                                        {rendering.english ?? '—'}
                                      </Text>
                                    </Table.Td>
                                    <Table.Td>{rendering.source ?? '—'}</Table.Td>
                                    <Table.Td>{rendering.relation ?? '—'}</Table.Td>
                                    <Table.Td>{confidenceLabel(rendering.confidence)}</Table.Td>
                                  </Table.Tr>
                                ))}
                              </Table.Tbody>
                            </Table>
                          </Table.ScrollContainer>
                        ) : (
                          <Text size="sm" c="dimmed">
                            No lexical renderings recorded for this group.
                          </Text>
                        )}

                        {evidence.length > 0 ? (
                          <Stack gap="xs">
                            <Text size="xs" c="dimmed" fw={500} tt="uppercase">
                              Matched Translation Evidence
                            </Text>
                            <Table.ScrollContainer minWidth={720}>
                              <Table striped highlightOnHover withTableBorder>
                                <Table.Thead>
                                  <Table.Tr>
                                    <Table.Th>Translator</Table.Th>
                                    <Table.Th>Matched span</Table.Th>
                                    <Table.Th>Rendering</Table.Th>
                                    <Table.Th>Method</Table.Th>
                                    <Table.Th>Confidence</Table.Th>
                                  </Table.Tr>
                                </Table.Thead>
                                <Table.Tbody>
                                  {evidence.map((alignment) => (
                                    <Table.Tr key={alignment.id ?? `${alignment.translator?.shortName}-${alignment.targetSpan}`}>
                                      <Table.Td>{alignment.translator?.shortName ?? alignment.translator?.name ?? '—'}</Table.Td>
                                      <Table.Td>{alignment.targetSpan ?? '—'}</Table.Td>
                                      <Table.Td>{alignment.rendering?.english ?? '—'}</Table.Td>
                                      <Table.Td>{alignment.method ?? '—'}</Table.Td>
                                      <Table.Td>{confidenceLabel(alignment.confidence)}</Table.Td>
                                    </Table.Tr>
                                  ))}
                                </Table.Tbody>
                              </Table>
                            </Table.ScrollContainer>
                          </Stack>
                        ) : null}
                      </Stack>
                    </Card>
                  );
                })}
              </Stack>
            </Stack>
          </Card>
        ) : null}

        {/* Scoring Evidence */}
        {allEvidence.length > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="sm">
              <Text fw={600} size="sm">
                Scoring Evidence — {allEvidence.length} match(es)
              </Text>
              <Table.ScrollContainer minWidth={760}>
                <Table striped highlightOnHover withTableBorder>
                  <Table.Thead>
                    <Table.Tr>
                      <Table.Th>Source group</Table.Th>
                      <Table.Th>Translator</Table.Th>
                      <Table.Th>Matched span</Table.Th>
                      <Table.Th>Rendering</Table.Th>
                      <Table.Th>Method</Table.Th>
                      <Table.Th>Confidence</Table.Th>
                    </Table.Tr>
                  </Table.Thead>
                  <Table.Tbody>
                    {allEvidence.map(({ group, alignment }) => (
                      <Table.Tr key={`${group.id}-${alignment.id}`}>
                        <Table.Td>
                          <Text size="sm">
                            {group.interlinearGloss ?? group.coptic ?? '—'}
                          </Text>
                        </Table.Td>
                        <Table.Td>{alignment.translator?.shortName ?? alignment.translator?.name ?? '—'}</Table.Td>
                        <Table.Td>{alignment.targetSpan ?? '—'}</Table.Td>
                        <Table.Td>{alignment.rendering?.english ?? '—'}</Table.Td>
                        <Table.Td>{alignment.method ?? '—'}</Table.Td>
                        <Table.Td>{confidenceLabel(alignment.confidence)}</Table.Td>
                      </Table.Tr>
                    ))}
                  </Table.Tbody>
                </Table>
              </Table.ScrollContainer>
            </Stack>
          </Card>
        ) : null}

        {/* Translation Equivalents */}
        {equivalents.length > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="sm">
              <Text fw={600} size="sm">
                English Renderings — Total: {totalEquivalentCount}
              </Text>
              <Group gap="xs">
                {equivalents.map((eq, i) => (
                  <Badge key={eq?.id ?? `${eq?.english}-${i}`} variant="light" color="gray">
                    {eq?.english}
                    {eq?.count ? ` x${eq.count}` : ''}
                    {eq?.source ? ` · ${sourceLabel(eq.source)}` : ''}
                  </Badge>
                ))}
              </Group>
            </Stack>
          </Card>
        ) : null}

        {/* Outline of English Usage */}
        {usageOutlineRows.length > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="sm">
              <Text fw={600} size="sm">
                Outline of English Usage
              </Text>
              <Stack gap={6}>
                {usageOutlineRows.map((row, index) => (
                  <Group
                    key={`${row.level}-${row.text}-${index}`}
                    gap="sm"
                    align="flex-start"
                    wrap="nowrap"
                    pl={row.level === 'minor' ? 'xl' : row.level === 'source' ? 'md' : 0}
                  >
                    <Text
                      size={row.level === 'major' ? 'md' : 'sm'}
                      c="dimmed"
                      fs={row.level === 'major' ? 'italic' : undefined}
                      style={{ width: 44, textAlign: 'right', flex: '0 0 44px' }}
                    >
                      {row.marker}
                    </Text>
                    <Stack gap={2}>
                      <Text size={row.level === 'major' ? 'md' : 'sm'} fw={row.level === 'major' ? 500 : 400}>
                        {row.text}
                      </Text>
                      {row.level !== 'source' ? (
                        <Text size="xs" c="dimmed">
                          {sourceLabel(row.source)}
                          {row.headword ? ` · ${row.headword}` : ''}
                        </Text>
                      ) : null}
                    </Stack>
                  </Group>
                ))}
              </Stack>
            </Stack>
          </Card>
        ) : null}

        {/* Definitions by Source */}
        {definitionsBySource.size > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="md">
              <Text fw={600} size="sm">
                Dictionary Source Details
              </Text>
              {Array.from(definitionsBySource.entries()).map(([source, defs]) => (
                <Stack key={source} gap="xs">
                  <Text fw={500} size="sm">
                    {sourceLabel(source)}
                  </Text>
                  {defs.map((def, i) => (
                    <Stack key={def?.id ?? i} gap="xs">
                      {def?.dictionaryHeadword ? (
                        <Text size="sm" c="dimmed">
                          Headword: {def.dictionaryHeadword}
                        </Text>
                      ) : null}
                      {definitionEnglishSenses(def).length > 0 ? (
                        <Stack gap="xs" pl="md">
                          {definitionEnglishSenses(def)
                            .map((sense) => (
                              <Text key={sense} size="sm" c="dimmed">
                                • {sense}
                              </Text>
                            ))}
                        </Stack>
                      ) : null}
                      {i < defs.length - 1 ? <Divider /> : null}
                    </Stack>
                  ))}
                </Stack>
              ))}
            </Stack>
          </Card>
        ) : null}

        {/* Usage Examples (Concordance) */}
        {usageExamples.length > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="sm">
              <Text fw={600} size="sm">
                Usage Examples — {usageExamples.length} occurrence(s)
              </Text>
              <Table.ScrollContainer minWidth={720}>
                <Table striped highlightOnHover withTableBorder>
                  <Table.Thead>
                    <Table.Tr>
                      <Table.Th>Logion</Table.Th>
                      <Table.Th>Coptic</Table.Th>
                      <Table.Th>Gloss</Table.Th>
                      <Table.Th>Translator</Table.Th>
                      <Table.Th>Rendering</Table.Th>
                    </Table.Tr>
                  </Table.Thead>
                  <Table.Tbody>
                    {usageExamples.map((ex, i) => (
                      <Table.Tr key={ex?.id ?? i}>
                        <Table.Td>
                          <Text fw={500} size="sm">
                            {ex?.logion?.number ?? '—'}
                          </Text>
                        </Table.Td>
                        <Table.Td>
                          <Text ff="serif" size="sm">
                            {ex?.coptic ?? '—'}
                          </Text>
                        </Table.Td>
                        <Table.Td>{ex?.interlinearGloss ?? '—'}</Table.Td>
                        <Table.Td>{ex?.translatorName ?? '—'}</Table.Td>
                        <Table.Td>{ex?.translatorRendering ?? '—'}</Table.Td>
                      </Table.Tr>
                    ))}
                  </Table.Tbody>
                </Table>
              </Table.ScrollContainer>
            </Stack>
          </Card>
        ) : null}

        {/* Dictionary Matches */}
        {lexicon?.dictionaryMatches && lexicon.dictionaryMatches.length > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="sm">
              <Text fw={600} size="sm">
                Raw Dictionary Lookup Candidates — {lexicon.dictionaryMatches.length} match(es)
              </Text>
              <Table striped highlightOnHover withTableBorder>
                <Table.Thead>
                  <Table.Tr>
                    <Table.Th>Headword</Table.Th>
                    <Table.Th>Part of Speech</Table.Th>
                    <Table.Th>Frequency</Table.Th>
                    <Table.Th>Source</Table.Th>
                    <Table.Th>Senses</Table.Th>
                  </Table.Tr>
                </Table.Thead>
                <Table.Tbody>
                  {lexicon.dictionaryMatches.map((match, i) => (
                    <Table.Tr key={match?.id ?? i}>
                      <Table.Td>
                        <Text fw={500} size="sm">
                          {match?.headword ?? '—'}
                        </Text>
                      </Table.Td>
                      <Table.Td>{match?.partOfSpeech ?? '—'}</Table.Td>
                      <Table.Td>{match?.frequency ?? '—'}</Table.Td>
                      <Table.Td>
                        <Badge size="sm" variant="light">
                          {match?.source ?? '—'}
                        </Badge>
                      </Table.Td>
                      <Table.Td>
                        {match?.senses && match.senses.length > 0 ? (
                          <Stack gap={2}>
                            {match.senses
                              .filter((s): s is string => s !== null)
                              .map((sense, j) => (
                                <Text key={j} size="sm">
                                  {sense}
                                </Text>
                              ))}
                          </Stack>
                        ) : (
                          '—'
                        )}
                      </Table.Td>
                    </Table.Tr>
                  ))}
                </Table.Tbody>
              </Table>
            </Stack>
          </Card>
        ) : null}

        {/* Translation Range */}
        {lexicon?.translationRange ? (
          <Card withBorder padding="md">
            <Stack gap="sm">
              <Text fw={600} size="sm">
                Translation Range
              </Text>
              <SimpleGrid cols={{ base: 1, sm: 3 }}>
                <Stack gap="xs">
                  <Text size="xs" c="dimmed" fw={500} tt="uppercase">
                    Literal
                  </Text>
                  {lexicon.translationRange.literal && lexicon.translationRange.literal.length > 0 ? (
                    lexicon.translationRange.literal
                      .filter((l): l is string => l !== null)
                      .map((l, i) => (
                        <Text key={i} size="sm">
                          {l}
                        </Text>
                      ))
                  ) : (
                    <Text size="sm" c="dimmed">—</Text>
                  )}
                </Stack>
                <Stack gap="xs">
                  <Text size="xs" c="dimmed" fw={500} tt="uppercase">
                    Acceptable
                  </Text>
                  {lexicon.translationRange.acceptable && lexicon.translationRange.acceptable.length > 0 ? (
                    lexicon.translationRange.acceptable
                      .filter((a): a is string => a !== null)
                      .map((a, i) => (
                        <Text key={i} size="sm">
                          {a}
                        </Text>
                      ))
                  ) : (
                    <Text size="sm" c="dimmed">—</Text>
                  )}
                </Stack>
                <Stack gap="xs">
                  <Text size="xs" c="dimmed" fw={500} tt="uppercase">
                    Not This Unit
                  </Text>
                  {lexicon.translationRange.notThisUnit && lexicon.translationRange.notThisUnit.length > 0 ? (
                    lexicon.translationRange.notThisUnit
                      .filter((n): n is string => n !== null)
                      .map((n, i) => (
                        <Text key={i} size="sm">
                          {n}
                        </Text>
                      ))
                  ) : (
                    <Text size="sm" c="dimmed">—</Text>
                  )}
                </Stack>
              </SimpleGrid>
            </Stack>
          </Card>
        ) : null}

        {/* Warnings */}
        {lexicon?.warnings && lexicon.warnings.length > 0 ? (
          <Card withBorder padding="md">
            <Stack gap="xs">
              <Text fw={600} size="sm" c="orange">
                Warnings
              </Text>
              {lexicon.warnings
                .filter((w): w is string => w !== null)
                .map((warning, i) => (
                  <Text key={i} size="sm" c="orange">
                    • {warning}
                  </Text>
                ))}
            </Stack>
          </Card>
        ) : null}

        {/* No Data Alert */}
        {!hasLexeme && !hasLexicon && sourceGroups.length === 0 ? (
          <Alert color="blue" title="No dictionary data">
            This token has no linked lexeme or lexicon entries in the database.
          </Alert>
        ) : null}
      </Stack>
    </Container>
  );
}
