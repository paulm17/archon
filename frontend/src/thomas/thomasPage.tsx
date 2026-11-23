import { useEffect, useState } from 'react';
import {
  Alert,
  Button,
  Container,
  Drawer,
  Group,
  Loader,
  Stack,
  Table,
  Tabs,
  Text,
  Textarea,
  TextInput,
  Tree,
  useTree,
  type TreeNodeData,
} from '@mantine/core';
import { useDebouncedState } from '@mantine/hooks';
import { executeVardaRequest } from '../lib/client';
import { useThomasStore } from '../lib/thomasStore';
import { openChildWindow } from '../lib/nativeWindow';
import { Scorecard } from './scorecard';

type ThomasTokenRecord = {
  id: string;
  tokenIndex: number;
  coptic: string | null;
  greek: string | null;
  transliteration: string | null;
  englishGloss: string | null;
  source: string | null;
  grammar: string | null;
};

type ThomasTranslationRecord = {
  uid: string;
  text: string;
  sourceLang: string | null;
  translatorUid: string;
  translatorName: string;
  translatorShortName: string;
};

type LogionRecord = {
  uid: string;
  number: string;
  copticText: string | null;
  notes: string | null;
  tokens: ThomasTokenRecord[];
  translations: ThomasTranslationRecord[];
};

type LogionQueryResult = {
  getLogion?: {
    id?: string | null;
    number?: string | null;
    copticText?: string | null;
    notes?: string | null;
    tokens?: Array<{
      id?: string | null;
      tokenIndex?: number | null;
      coptic?: string | null;
      greek?: string | null;
      transliteration?: string | null;
      englishGloss?: string | null;
      source?: string | null;
      morphology?: {
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
        morphologyClass?: string | null;
        caseVal?: string | null;
      } | null;
    } | null> | null;
    translations?: Array<{
      id?: string | null;
      text?: string | null;
      sourceLang?: string | null;
      translator?: {
        id?: string | null;
        name?: string | null;
        shortName?: string | null;
      } | null;
    } | null> | null;
  } | null;
};

type LogionNumbersQueryResult = {
  queryLogion?: Array<{
    id?: string | null;
    number?: string | null;
  } | null> | null;
};

type CreateTranslationResult = {
  createTranslation?: {
    id?: string | null;
  } | null;
};

function formatGrammar(morphology: {
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
  morphologyClass?: string | null;
  caseVal?: string | null;
} | null): string | null {
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

function normalizeLogion(result: LogionQueryResult): LogionRecord | null {
  const logion = result.getLogion;
  if (!logion?.id || !logion?.number) {
    return null;
  }

  const tokens: ThomasTokenRecord[] = (logion.tokens ?? [])
    .flatMap((token) => {
      if (typeof token?.tokenIndex !== 'number' || !token?.id) return [];
      return [{
        id: token.id,
        tokenIndex: token.tokenIndex,
        coptic: token.coptic ?? null,
        greek: token.greek ?? null,
        transliteration: token.transliteration ?? null,
        englishGloss: token.englishGloss ?? null,
        source: token.source ?? null,
        grammar: formatGrammar(token.morphology ?? null),
      }];
    })
    .sort((a, b) => a.tokenIndex - b.tokenIndex);

  const translations: ThomasTranslationRecord[] = (logion.translations ?? [])
    .flatMap((t) => {
      if (!t?.id || !t?.text || !t?.translator?.name) return [];
      return [{
        uid: t.id,
        text: t.text,
        sourceLang: t.sourceLang ?? null,
        translatorUid: t.translator.id,
        translatorName: t.translator.name,
        translatorShortName: t.translator.shortName ?? t.translator.name,
      }];
    });

  return {
    uid: logion.id,
    number: logion.number,
    copticText: logion.copticText ?? null,
    notes: logion.notes ?? null,
    tokens,
    translations,
  };
}

async function fetchLogionNumbers(): Promise<{ numbers: string[]; numberToUid: Map<string, string> }> {
  const result = await executeVardaRequest<LogionNumbersQueryResult>(
    'thomas',
    `
      query GetThomasLogionNumbers {
        queryLogion {
          id
          number
        }
      }
    `,
  );

  const entries = (result.queryLogion ?? [])
    .flatMap((l) => (l?.id && l?.number ? [{ id: l.id, number: l.number }] : []));

  const numbers = entries
    .map((e) => e.number)
    .sort((a, b) => {
      const na = parseInt(a, 10);
      const nb = parseInt(b, 10);
      if (Number.isNaN(na) && Number.isNaN(nb)) return a.localeCompare(b);
      if (Number.isNaN(na)) return -1;
      if (Number.isNaN(nb)) return 1;
      return na - nb;
    });

  const numberToUid = new Map<string, string>();
  for (const entry of entries) {
    numberToUid.set(entry.number, entry.id);
  }

  return { numbers, numberToUid };
}

async function fetchLogionByUid(uid: string): Promise<LogionRecord | null> {
  const result = await executeVardaRequest<LogionQueryResult>(
    'thomas',
    `
      query GetThomasLogion($uid: ID!) {
        getLogion(uid: $uid) {
          id
          number
          copticText
          notes
          tokens(first: 200) {
            id
            tokenIndex
            coptic
            greek
            transliteration
            englishGloss
            source
            lexeme {
              id
              headword
              transliteration
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
              morphologyClass
              caseVal
            }
          }
          translations(first: 50) {
            id
            text
            sourceLang
            translator {
              id
              name
              shortName
            }
          }
        }
      }
    `,
    { uid },
  );

  return normalizeLogion(result);
}

async function saveLogionNotes(uid: string, input: { notes: string | null }): Promise<void> {
  const result = await executeVardaRequest<{ updateLogion?: boolean | null }>(
    'thomas',
    `
      mutation UpdateThomasLogion($uid: ID!, $input: LogionInput!) {
        updateLogion(uid: $uid, input: $input)
      }
    `,
    { uid, input },
  );

  if (!result.updateLogion) {
    throw new Error('Save failed.');
  }
}

async function saveTranslation(
  uid: string,
  input: { text: string; sourceLang: string | null },
): Promise<void> {
  const result = await executeVardaRequest<{ updateTranslation?: boolean | null }>(
    'thomas',
    `
      mutation UpdateThomasTranslation($uid: ID!, $input: TranslationInput!) {
        updateTranslation(uid: $uid, input: $input)
      }
    `,
    { uid, input: { ...input, uid } },
  );

  if (!result.updateTranslation) {
    throw new Error('Save failed.');
  }
}

async function saveNewTranslation(
  input: { text: string; sourceLang: string | null; logionUid: string; translatorUid: string },
): Promise<string> {
  const result = await executeVardaRequest<CreateTranslationResult>(
    'thomas',
    `
      mutation AddThomasTranslation($input: TranslationInput!) {
        createTranslation(input: $input) {
          id
        }
      }
    `,
    {
      input: {
        text: input.text,
        sourceLang: input.sourceLang,
        logion: { id: input.logionUid },
        translator: { id: input.translatorUid },
      },
    },
  );

  if (!result.createTranslation?.id) {
    throw new Error('Save failed.');
  }

  return result.createTranslation.id;
}

async function fetchPaulMossTranslatorUid(): Promise<string | null> {
  const result = await executeVardaRequest<{ queryTranslator?: Array<{ id?: string | null } | null> | null }>(
    'thomas',
    `
      query GetPaulMossTranslator {
        queryTranslator(filter: { name: { eq: "Paul Moss" } }) {
          id
        }
      }
    `,
  );
  return result.queryTranslator?.[0]?.id ?? null;
}

const FIRST_COLUMN_WIDTH_PX = 400;

export function ThomasPage() {
  const [logionNumbers, setLogionNumbers] = useState<string[]>([]);
  const [numberToUid, setNumberToUid] = useState<Map<string, string>>(new Map());
  const { currentLogionNumber, setCurrentLogionNumber } = useThomasStore();
  const [logion, setLogion] = useState<LogionRecord | null>(null);
  const [translationDraft, setTranslationDraft] = useState('');
  const [notesDraft, setNotesDraft] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saveMessage, setSaveMessage] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<string>('logion');
  const [drawerOpened, setDrawerOpened] = useState<boolean>(false);
  const [filterInput, setFilterInput] = useState('');
  const [debouncedFilter, setDebouncedFilter] = useDebouncedState('', 200);
  const [paulMossTranslatorUid, setPaulMossTranslatorUid] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadMeta() {
      try {
        setLoading(true);
        setError(null);

        const { numbers, numberToUid: uidMap } = await fetchLogionNumbers();
        if (cancelled) return;

        setLogionNumbers(numbers);
        setNumberToUid(uidMap);

        const translatorUid = await fetchPaulMossTranslatorUid();
        if (!cancelled) {
          setPaulMossTranslatorUid(translatorUid);
        }
      } catch (loadError) {
        if (!cancelled) {
          console.error(loadError);
          setError('Failed to load Thomas logion list.');
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

  const currentIndex = logionNumbers.indexOf(currentLogionNumber);
  const lastLogionNumber = logionNumbers.length > 0 ? logionNumbers[logionNumbers.length - 1] : '1';

  useEffect(() => {
    if (logionNumbers.length === 0) return;

    const uid = numberToUid.get(currentLogionNumber);
    if (!uid) {
      setError(`Logion ${currentLogionNumber} has no UID mapping.`);
      setLoading(false);
      return;
    }

    let cancelled = false;

    async function loadLogionData() {
      try {
        setLoading(true);
        setError(null);
        setSaveMessage(null);

        const loaded = await fetchLogionByUid(uid);
        if (cancelled) return;

        if (!loaded) {
          throw new Error(`Logion ${currentLogionNumber} was not found.`);
        }

        setLogion(loaded);

        const paulMossTranslation = loaded.translations.find(
          (t) => t.translatorName.trim().toLowerCase() === 'paul moss',
        );
        setTranslationDraft(paulMossTranslation?.text ?? '');
        setNotesDraft(loaded.notes ?? '');
      } catch (loadError) {
        if (!cancelled) {
          console.error(loadError);
          setError('Failed to load Thomas logion.');
          setLogion(null);
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    void loadLogionData();

    return () => {
      cancelled = true;
    };
  }, [logionNumbers, currentLogionNumber, numberToUid]);

  const paulMossTranslation = logion?.translations.find(
    (t) => t.translatorName.trim().toLowerCase() === 'paul moss',
  );
  const otherTranslations = logion?.translations.filter(
    (t) => t.translatorName.trim().toLowerCase() !== 'paul moss',
  ) ?? [];

  const paulMossOriginalText = paulMossTranslation?.text ?? '';
  const hasUnsavedChanges =
    logion !== null &&
    (translationDraft !== paulMossOriginalText ||
      notesDraft !== (logion.notes ?? ''));

  async function handleSave() {
    if (!logion || saving) return;

    try {
      setSaving(true);
      setError(null);
      setSaveMessage(null);

      let newTranslationUid: string | null = null;

      const promises: Promise<void>[] = [];

      if (paulMossTranslation) {
        if (translationDraft !== paulMossTranslation.text) {
          promises.push(
            saveTranslation(paulMossTranslation.uid, {
              text: translationDraft,
              sourceLang: paulMossTranslation.sourceLang,
            }),
          );
        }
      } else if (translationDraft.trim() && paulMossTranslatorUid) {
        promises.push(
          saveNewTranslation({
            text: translationDraft,
            sourceLang: null,
            logionUid: logion.uid,
            translatorUid: paulMossTranslatorUid,
          }).then((dbUid) => {
            newTranslationUid = dbUid;
          }),
        );
      }

      if (notesDraft !== (logion.notes ?? '')) {
        promises.push(
          saveLogionNotes(logion.uid, {
            notes: notesDraft.trim() ? notesDraft : null,
          }),
        );
      }

      await Promise.all(promises);

      setLogion({
        ...logion,
        notes: notesDraft.trim() ? notesDraft : null,
        translations: newTranslationUid
          ? [
              ...logion.translations,
              {
                uid: newTranslationUid,
                text: translationDraft,
                sourceLang: null,
                translatorUid: paulMossTranslatorUid!,
                translatorName: 'Paul Moss',
                translatorShortName: 'Paul Moss',
              },
            ]
          : logion.translations.map((t) =>
              t.translatorName.trim().toLowerCase() === 'paul moss'
                ? { ...t, text: translationDraft }
                : t,
            ),
      });
      setSaveMessage('Saved.');
    } catch (saveError) {
      console.error(saveError);
      setError('Failed to save.');
    } finally {
      setSaving(false);
    }
  }

  const sortedOtherTranslations = [...otherTranslations].sort((a, b) =>
    a.translatorName.localeCompare(b.translatorName),
  );

  const translationTableRows = sortedOtherTranslations.map((t, index, arr) => {
    const previousRow = index > 0 ? arr[index - 1] : null;
    const showTranslator = !previousRow || previousRow.translatorName !== t.translatorName;

    return (
      <Table.Tr key={t.uid}>
        <Table.Td style={{ width: FIRST_COLUMN_WIDTH_PX, minWidth: FIRST_COLUMN_WIDTH_PX }}>
          {showTranslator ? <Text fw={600}>{t.translatorName}</Text> : null}
        </Table.Td>
        <Table.Td>
          <Text span>{t.text}</Text>
        </Table.Td>
      </Table.Tr>
    );
  });

  const interlinearBlocks = logion?.tokens.map((token) => {
    return (
      <div
        key={`token-${token.tokenIndex}`}
        onClick={() => {
          console.log('[Thomas] Clicked token:', token.id, token.coptic, token.englishGloss);
          try {
            openChildWindow({
              title: `Dictionary — ${token.coptic || token.englishGloss || 'Token'}`,
              url: `/dictionary/${token.id}?child=1`,
              width: 800,
              height: 900,
              persist_key: `dict_${token.id}`,
            });
          } catch (err) {
            console.error('[Thomas] Failed to open dictionary window:', err);
          }
        }}
        style={{
          minWidth: 72,
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
          whiteSpace: 'nowrap',
          cursor: 'pointer',
          padding: '2px 4px',
          borderRadius: 4,
          transition: 'background-color 0.15s',
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLDivElement).style.backgroundColor = 'var(--mantine-color-gray-1)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLDivElement).style.backgroundColor = 'transparent';
        }}
      >
        <Text
          ff="serif"
          size="1.75rem"
          style={{ lineHeight: 1, color: 'var(--mantine-color-dark-9)' }}
        >
          {token.coptic || '\u00A0'}
        </Text>
        <Text c="orange.7" fw={500} size="1rem" style={{ lineHeight: 1.1 }}>
          {token.englishGloss || '\u00A0'}
        </Text>
      </div>
    );
  });

  const tree = useTree();

  function buildLogionTree(): TreeNodeData[] {
    return logionNumbers.map((number) => ({
      value: `logion-${number}`,
      label: `Logion ${number}`,
      nodeProps: {
        logionNumber: number,
      },
    }));
  }

  function filterLogionTree(nodes: TreeNodeData[], filterValue: string): TreeNodeData[] {
    const normalizedFilter = filterValue.trim().toLowerCase();
    if (!normalizedFilter) return nodes;
    return nodes.filter((node) => {
      const nodeLabel = typeof node.label === 'string' ? node.label.toLowerCase() : '';
      return nodeLabel.includes(normalizedFilter);
    });
  }

  const logionTree = buildLogionTree();
  const filteredLogionTree = filterLogionTree(logionTree, debouncedFilter);

  return (
    <Tabs
      value={activeTab}
      onChange={setActiveTab}
      style={{ flex: 1, minHeight: 0, height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}
    >
      <Tabs.List>
        <Tabs.Tab value="logion">Logion</Tabs.Tab>
        <Tabs.Tab value="scorecard">Scorecard</Tabs.Tab>
      </Tabs.List>

      <Tabs.Panel value="logion" style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
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
        title="Logions"
        position="right"
        size="md"
      >
        <Stack gap="sm">
          <TextInput
            placeholder="Filter logions"
            value={filterInput}
            onChange={(event) => {
              const nextValue = event.currentTarget.value;
              setFilterInput(nextValue);
              setDebouncedFilter(nextValue);
            }}
          />
          <Tree
            data={filteredLogionTree}
            tree={tree}
            selectOnClick
            clearSelectionOnOutsideClick
            renderNode={({ node, elementProps }) => (
              <Group
                gap="xs"
                {...elementProps}
                onClick={(event) => {
                  elementProps.onClick(event);
                  const logionNumber = node.nodeProps?.logionNumber;
                  if (typeof logionNumber === 'string') {
                    setCurrentLogionNumber(logionNumber);
                    setDrawerOpened(false);
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
                Logion {currentLogionNumber} of {lastLogionNumber}
              </Text>
              <Button
                variant="light"
                size="compact-sm"
                onClick={() => setDrawerOpened(true)}
              >
                B
              </Button>
            </Group>
            <Button onClick={() => void handleSave()} disabled={!hasUnsavedChanges || loading || saving}>
              {saving ? 'Saving...' : 'Save'}
            </Button>
          </Group>
        </Stack>

        {error ? (
          <Alert color="red" title="Error">
            {error}
          </Alert>
        ) : null}

        {saveMessage ? (
          <Alert color="green" title="Saved">
            {saveMessage}
          </Alert>
        ) : null}

        {!loading && logion ? (
          <Textarea
            label="Translation"
            value={translationDraft}
            onChange={(event) => setTranslationDraft(event.currentTarget.value)}
            autosize
            minRows={4}
          />
        ) : null}

        {!loading && logion ? (
          <Textarea
            label="Notes"
            value={notesDraft}
            onChange={(event) => setNotesDraft(event.currentTarget.value)}
            autosize
            minRows={4}
          />
        ) : null}

        <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
          {loading ? (
            <Group justify="center" py="xl">
              <Loader />
            </Group>
          ) : null}

          {!loading && logion ? (
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
                <div style={{ flex: 1 }}>Text</div>
              </Group>
              <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden' }}>
                {logion.tokens.length > 0 ? (
                  <div
                    style={{
                      borderLeft: '1px solid var(--mantine-color-gray-3)',
                      borderRight: '1px solid var(--mantine-color-gray-3)',
                      borderTop: '1px solid var(--mantine-color-gray-3)',
                      marginBottom: 8,
                    }}
                  >
                    <div
                      style={{
                        display: 'flex',
                        borderBottom: '1px solid var(--mantine-color-gray-3)',
                      }}
                    >
                      <div style={{ width: FIRST_COLUMN_WIDTH_PX, minWidth: FIRST_COLUMN_WIDTH_PX, padding: '10px 12px', fontWeight: 600 }}>
                        Interlinear
                      </div>
                      <div style={{ flex: 1, minWidth: 0, paddingTop: 10, paddingBottom: 14 }}>
                        <div style={{ overflowX: 'auto', overflowY: 'hidden' }}>
                          <div style={{ display: 'inline-flex', alignItems: 'flex-start', gap: 18, minWidth: 'max-content', paddingRight: 12 }}>
                            {interlinearBlocks}
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                ) : null}
                {sortedOtherTranslations.length > 0 ? (
                  <Table striped highlightOnHover>
                    <Table.Tbody>{translationTableRows}</Table.Tbody>
                  </Table>
                ) : (
                  <Alert color="blue" title="No translations">
                    No translations were found for this logion.
                  </Alert>
                )}
              </div>
            </div>
          ) : null}
        </div>

        <Group justify="space-between" grow>
          <Button
            variant="default"
            fullWidth
            disabled={currentIndex <= 0 || loading}
            onClick={() => {
              if (currentIndex > 0) {
                setCurrentLogionNumber(logionNumbers[currentIndex - 1]);
              }
            }}
          >
            Previous
          </Button>
          <Button
            fullWidth
            disabled={currentIndex >= logionNumbers.length - 1 || loading}
            onClick={() => {
              if (currentIndex < logionNumbers.length - 1) {
                setCurrentLogionNumber(logionNumbers[currentIndex + 1]);
              }
            }}
          >
            Next
          </Button>
        </Group>
      </Stack>
    </Container>
      </Tabs.Panel>

      <Tabs.Panel value="scorecard" style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        <Container size="100%" px={0} py="md">
          <Scorecard logionUid={logion?.uid ?? null} />
        </Container>
      </Tabs.Panel>
    </Tabs>
  );
}
