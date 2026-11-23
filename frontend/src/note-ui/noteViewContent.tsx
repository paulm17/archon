import { useEffect, useMemo, useState } from 'react';
import {
  ActionIcon,
  Alert,
  Badge,
  Box,
  Button,
  Group,
  Image,
  Pagination,
  Paper,
  Popover,
  SimpleGrid,
  Stack,
  Tabs,
  Text,
  Textarea,
  TextInput,
  ThemeIcon,
  Title,
  UnstyledButton,
} from '@mantine/core';
import { IconAlertCircle, IconLink, IconPhoto, IconX } from '@tabler/icons-react';
import MDEditor from '@uiw/react-md-editor';
import { openChildWindow } from '../lib/nativeWindow';
import { fetchVerses, getNote, listRelatedNotes, type NoteRecord, type RelatedNoteRecord, type VerseLookupResult } from './api';
import { isValidVerseReference, VERSE_INPUT_PATTERN } from './verseReferences';

type VerseCache = Record<string, VerseLookupResult[]>;
const RAW_URL_RE = /https?:\/\/[^\s<>"')\]]+/g;
const RELATED_NOTES_PAGE_SIZE = 10;

function parseVerseRange(fullRef: string) {
  const match = fullRef.match(VERSE_INPUT_PATTERN);
  if (!match) {
    return null;
  }
  return {
    gospel: match[1].replace(/\s+/g, ' ').trim(),
    chapter: Number(match[2]),
    start: Number(match[3]),
    end: Number(match[4] ?? match[3]),
  };
}

function extractUrlsFromContent(content: string) {
  return Array.from(new Set(content.match(RAW_URL_RE) ?? []));
}

function relationshipLabel(relationshipType: string) {
  switch (relationshipType) {
    case 'NOTE_RELATED_BY_AI_SECOND_PASS':
      return 'AI related';
    case 'NOTE_SHARES_VERSE':
      return 'Shared verse';
    default:
      return relationshipType;
  }
}

type NoteViewContentProps = {
  noteId: string;
  embedded?: boolean;
  onClose?: () => void;
  onOpenNote?: (noteRecord: RelatedNoteRecord) => void | Promise<void>;
};

export function NoteViewContent({ noteId, embedded = false, onClose, onOpenNote }: NoteViewContentProps) {
  const [note, setNote] = useState<NoteRecord | null>(null);
  const [relatedNotes, setRelatedNotes] = useState<RelatedNoteRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeVerse, setActiveVerse] = useState<string | null>(null);
  const [showSummary, setShowSummary] = useState(false);
  const [verseLookupError, setVerseLookupError] = useState<string | null>(null);
  const [verseCache, setVerseCache] = useState<VerseCache>({});
  const [relatedNotesPage, setRelatedNotesPage] = useState(1);

  const viewportHeight = embedded ? '100%' : 'calc(100vh - 210px)';
  const displayUrls = useMemo(() => {
    if (!note) {
      return [];
    }
    const noteVideoUrls = note.noteVideos.map((video) => video.url);
    return note.urls.length > 0 ? note.urls : Array.from(new Set([...noteVideoUrls, ...extractUrlsFromContent(note.content)]));
  }, [note]);
  const visibleVerses = useMemo(
    () => note?.noteVerses.filter((verse) => isValidVerseReference(verse.fullRef)) ?? [],
    [note],
  );
  const contextualRelatedNotes = useMemo(
    () => relatedNotes.filter((noteRecord) => noteRecord.relationshipType === 'NOTE_RELATED_BY_AI_SECOND_PASS'),
    [relatedNotes],
  );
  const noteConnections = useMemo(
    () => relatedNotes.filter((noteRecord) => noteRecord.relationshipType === 'NOTE_SHARES_VERSE'),
    [relatedNotes],
  );
  const relatedNotesTotalPages = Math.max(1, Math.ceil(contextualRelatedNotes.length / RELATED_NOTES_PAGE_SIZE));
  const pagedRelatedNotes = useMemo(() => {
    const start = (relatedNotesPage - 1) * RELATED_NOTES_PAGE_SIZE;
    return contextualRelatedNotes.slice(start, start + RELATED_NOTES_PAGE_SIZE);
  }, [contextualRelatedNotes, relatedNotesPage]);

  useEffect(() => {
    setVerseCache({});
    setActiveVerse(null);
    setShowSummary(false);
    setRelatedNotesPage(1);

    void (async () => {
      setLoading(true);
      try {
        const [loaded, related] = await Promise.all([getNote(noteId), listRelatedNotes(noteId)]);
        if (!loaded) {
          throw new Error('Note not found.');
        }
        setNote(loaded);
        setRelatedNotes(related);
        setError(null);
      } catch (loadError) {
        setError(loadError instanceof Error ? loadError.message : 'Failed to load note.');
      } finally {
        setLoading(false);
      }
    })();
  }, [noteId]);

  async function openVerse(fullRef: string) {
    setActiveVerse(fullRef);
    setVerseLookupError(null);
    if (verseCache[fullRef]) {
      return;
    }
    const parsed = parseVerseRange(fullRef);
    if (!parsed) {
      setVerseLookupError('Could not parse verse reference.');
      return;
    }
    try {
      const records = await fetchVerses(parsed.gospel, parsed.chapter, parsed.start, parsed.end);
      setVerseCache((current) => ({ ...current, [fullRef]: records }));
    } catch (lookupError) {
      setVerseLookupError(lookupError instanceof Error ? lookupError.message : 'Verse lookup failed.');
    }
  }

  async function handleOpenRelatedNote(noteRecord: RelatedNoteRecord) {
    if (onOpenNote) {
      await onOpenNote(noteRecord);
      return;
    }

    await openChildWindow({
      title: noteRecord.title,
      url: `${window.location.origin}/notes/${noteRecord.id}/view?child=1`,
      width: 1200,
      height: 900,
      persist_key: `notes-view-${noteRecord.id}`,
    });
  }

  return (
    <Stack gap="lg" style={embedded ? { height: '100%', minHeight: 0 } : undefined}>
      {onClose ? (
        <Group justify="space-between" align="center">
          <Title order={2}>View Note</Title>
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
        </Group>
      ) : null}

      {error && (
        <Alert color="red" icon={<IconAlertCircle size={16} />}>
          {error}
        </Alert>
      )}

      {!error && note ? (
        <SimpleGrid cols={{ base: 1, xl: 2 }} spacing="lg" style={embedded ? { flex: 1, minHeight: 0 } : undefined}>
          <Paper radius="xl" p="lg" withBorder style={{ height: viewportHeight, display: 'flex' }}>
            <Stack gap="md" style={{ flex: 1, minHeight: 0 }}>
              <Group grow>
                <TextInput label="Title" value={note.title} readOnly />
                <TextInput label="Category" value={note.category?.name ?? 'Uncategorised'} readOnly />
              </Group>

              {showSummary ? (
                <Box>
                  <Group justify="space-between" align="center" mb={6}>
                    <Text fw={500} size="sm">
                      Summary
                    </Text>
                    <ActionIcon variant="subtle" size="sm" aria-label="Close summary" onClick={() => setShowSummary(false)}>
                      <IconX size={14} />
                    </ActionIcon>
                  </Group>
                  <Textarea value={note.summary ?? ''} readOnly rows={5} />
                </Box>
              ) : (
                <Button variant="light" size="sm" w="fit-content" onClick={() => setShowSummary(true)}>
                  Show summary
                </Button>
              )}

              <Box style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
                <Text fw={500} size="sm" mb={6}>
                  Content
                </Text>
                <Box
                  style={{
                    flex: 1,
                    minHeight: 0,
                    overflow: 'auto',
                    border: '1px solid #ced4da',
                    borderRadius: 12,
                    padding: 12,
                  }}
                  data-color-mode="light"
                >
                  <MDEditor.Markdown source={note.content || ''} style={{ backgroundColor: 'transparent' }} />
                </Box>
              </Box>
            </Stack>
          </Paper>

          <Stack gap="lg" style={{ minHeight: viewportHeight }}>
            <Paper radius="xl" p="lg" withBorder>
              <Stack gap="md">
                <Group gap="sm">
                  <ThemeIcon variant="light" color="blue" radius="xl">
                    <IconLink size={16} />
                  </ThemeIcon>
                  <Title order={4}>URLs</Title>
                </Group>
                {displayUrls.length === 0 ? (
                  <Text c="dimmed">No URLs attached.</Text>
                ) : (
                  <Stack gap="xs">
                    {displayUrls.map((url) => (
                      <Text key={url} component="a" href={url} target="_blank" rel="noreferrer" c="blue">
                        {url}
                      </Text>
                    ))}
                  </Stack>
                )}
              </Stack>
            </Paper>

            {note.noteVideos.length > 0 ? (
              <Paper radius="xl" p="lg" withBorder>
                <Stack gap="md">
                  <Title order={4}>Script / Subtitles</Title>
                  <Tabs defaultValue={note.noteVideos[0]?.videoId}>
                    <Tabs.List>
                      {note.noteVideos.map((video) => (
                        <Tabs.Tab key={video.id} value={video.videoId}>
                          {video.videoId}
                        </Tabs.Tab>
                      ))}
                    </Tabs.List>
                    {note.noteVideos.map((video) => (
                      <Tabs.Panel key={video.id} value={video.videoId} pt="md">
                        <Stack gap="xs">
                          <Text style={{ whiteSpace: 'pre-wrap' }}>
                            {video.scriptContent?.trim() ? video.scriptContent : 'No transcript attached.'}
                          </Text>
                        </Stack>
                      </Tabs.Panel>
                    ))}
                  </Tabs>
                </Stack>
              </Paper>
            ) : null}

            <Paper radius="xl" p="lg" withBorder>
              <Stack gap="md">
                <Group gap="sm">
                  <ThemeIcon variant="light" color="orange" radius="xl">
                    <IconPhoto size={16} />
                  </ThemeIcon>
                  <Title order={4}>Images</Title>
                </Group>
                {note.images.length === 0 ? (
                  <Text c="dimmed">No images attached.</Text>
                ) : (
                  <Stack gap="sm">
                    {note.images.map((image) => (
                      <Image key={image} src={image} radius="md" alt="" />
                    ))}
                  </Stack>
                )}
              </Stack>
            </Paper>

            <Paper radius="xl" p="lg" withBorder style={{ flex: 1 }}>
              <Stack gap="md" style={{ height: '100%' }}>
                <Group justify="space-between">
                  <Title order={4}>Detected Verses</Title>
                  <Text c="dimmed" size="sm">
                    Read only
                  </Text>
                </Group>
                {visibleVerses.length === 0 ? (
                  <Text c="dimmed">No verse references detected for this note.</Text>
                ) : (
                  <Group gap="xs">
                    {visibleVerses.map((verse) => (
                      <Popover
                        key={verse.id}
                        width={360}
                        position="bottom-start"
                        opened={activeVerse === verse.fullRef}
                        onChange={(opened) => setActiveVerse(opened ? verse.fullRef : null)}
                      >
                        <Popover.Target>
                          <Badge
                            variant="light"
                            size="lg"
                            style={{ cursor: 'pointer' }}
                            onClick={() => void openVerse(verse.fullRef)}
                          >
                            {verse.fullRef}
                          </Badge>
                        </Popover.Target>
                        <Popover.Dropdown>
                          <Stack gap="xs">
                            <Text fw={600}>{verse.fullRef}</Text>
                            {verseLookupError && activeVerse === verse.fullRef ? <Text c="red">{verseLookupError}</Text> : null}
                            {(verseCache[verse.fullRef] ?? []).map((entry) => (
                              <Box key={`${verse.fullRef}-${entry.verseNumber}`}>
                                <Text fw={500}>{entry.verseNumber ?? '?'}</Text>
                                <Text size="sm">{entry.text ?? 'No verse text available.'}</Text>
                              </Box>
                            ))}
                            {!verseLookupError && !verseCache[verse.fullRef] ? <Text c="dimmed">Loading verses...</Text> : null}
                          </Stack>
                        </Popover.Dropdown>
                      </Popover>
                    ))}
                  </Group>
                )}
              </Stack>
            </Paper>

            <Paper radius="xl" p="lg" withBorder>
              <Stack gap="md">
                <Group justify="space-between" align="center">
                  <Title order={4}>Related Notes</Title>
                  <Text c="dimmed" size="sm">
                    Read only
                  </Text>
                </Group>
                {contextualRelatedNotes.length === 0 ? (
                  <Text c="dimmed">No related notes found for this note.</Text>
                ) : (
                  <Stack gap="sm">
                    {pagedRelatedNotes.map((related) => (
                      <UnstyledButton
                        key={`${related.id}-${related.relationshipType}`}
                        onClick={() => void handleOpenRelatedNote(related)}
                        style={{
                          textAlign: 'left',
                          border: '1px solid #ced4da',
                          borderRadius: 12,
                          padding: 12,
                        }}
                      >
                        <Stack gap={6}>
                          <Group justify="space-between" align="flex-start">
                            <Text fw={600}>{related.title}</Text>
                            <Badge variant="light">{relationshipLabel(related.relationshipType)}</Badge>
                          </Group>
                          {related.summary ? (
                            <Text size="sm" c="dimmed" lineClamp={3}>
                              {related.summary}
                            </Text>
                          ) : (
                            <Text size="sm" c="dimmed">
                              No summary available.
                            </Text>
                          )}
                        </Stack>
                      </UnstyledButton>
                    ))}
                    {contextualRelatedNotes.length > RELATED_NOTES_PAGE_SIZE ? (
                      <Group justify="space-between" align="center">
                        <Text c="dimmed" size="sm">
                          Showing {(relatedNotesPage - 1) * RELATED_NOTES_PAGE_SIZE + 1}-
                          {Math.min(relatedNotesPage * RELATED_NOTES_PAGE_SIZE, contextualRelatedNotes.length)} of {contextualRelatedNotes.length}
                        </Text>
                        <Pagination
                          total={relatedNotesTotalPages}
                          value={relatedNotesPage}
                          onChange={setRelatedNotesPage}
                          size="sm"
                        />
                      </Group>
                    ) : null}
                  </Stack>
                )}
              </Stack>
            </Paper>

            <Paper radius="xl" p="lg" withBorder>
              <Stack gap="md">
                <Group justify="space-between" align="center">
                  <Title order={4}>Connections</Title>
                  <Text c="dimmed" size="sm">
                    Shared verse overlap
                  </Text>
                </Group>
                {noteConnections.length === 0 ? (
                  <Text c="dimmed">No strong verse-based connections found for this note.</Text>
                ) : (
                  <Stack gap="sm">
                    {noteConnections.map((related) => (
                      <UnstyledButton
                        key={`${related.id}-${related.relationshipType}`}
                        onClick={() => void handleOpenRelatedNote(related)}
                        style={{
                          textAlign: 'left',
                          border: '1px solid #ced4da',
                          borderRadius: 12,
                          padding: 12,
                        }}
                      >
                        <Stack gap={6}>
                          <Group justify="space-between" align="flex-start">
                            <Text fw={600}>{related.title}</Text>
                            <Badge variant="light">{relationshipLabel(related.relationshipType)}</Badge>
                          </Group>
                          {related.summary ? (
                            <Text size="sm" c="dimmed" lineClamp={3}>
                              {related.summary}
                            </Text>
                          ) : (
                            <Text size="sm" c="dimmed">
                              No summary available.
                            </Text>
                          )}
                        </Stack>
                      </UnstyledButton>
                    ))}
                  </Stack>
                )}
              </Stack>
            </Paper>
          </Stack>
        </SimpleGrid>
      ) : null}

      {!error && !note && loading ? <Text c="dimmed">Loading note…</Text> : null}
    </Stack>
  );
}
