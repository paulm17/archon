import { useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import {
  ActionIcon,
  Alert,
  Badge,
  Box,
  Button,
  Card,
  Group,
  Image,
  Paper,
  Popover,
  Select,
  SimpleGrid,
  Stack,
  Tabs,
  Text,
  TextInput,
  Textarea,
  ThemeIcon,
  Title,
} from '@mantine/core';
import {
  IconAlertCircle,
  IconBold,
  IconCheck,
  IconCode,
  IconH1,
  IconItalic,
  IconLink,
  IconList,
  IconListNumbers,
  IconPhoto,
  IconPlus,
  IconQuote,
  IconRefresh,
  IconTrash,
  IconUpload,
} from '@tabler/icons-react';
import * as v from 'valibot';
import { checkDuplicateNoteTitle, fetchVerses, getNote, listCategories, saveNote, uploadImage, type NoteCategory, type NoteVerseInput, type NoteVideoRecord, type SaveNoteInput, type VerseLookupResult } from './api';
import { buildCategoryOptions } from './categoryTree';
import { extractVersesFromContent, VERSE_INPUT_PATTERN } from './verseReferences';

type UploadedImage = {
  id: string;
  file?: File;
  previewUrl: string;
  uploadedUrl: string;
  status: 'uploading' | 'done' | 'failed';
  error: string | null;
};

type VerseCache = Record<string, VerseLookupResult[]>;

type NoteFormState = {
  uid?: string;
  title: string;
  categoryId: string;
  content: string;
  urlInput: string;
  urls: string[];
  noteVideos: NoteVideoRecord[];
  images: UploadedImage[];
  createdAt?: string;
};

const titleSchema = v.pipe(v.string(), v.trim(), v.minLength(3, 'Title must be at least 3 characters.'));
const categorySchema = v.pipe(v.string(), v.trim(), v.minLength(1, 'Category is required.'));
const urlSchema = v.pipe(v.string(), v.trim(), v.url('URL must be a valid URL.'));
const noteSchema = v.object({
  title: titleSchema,
  categoryId: categorySchema,
  urls: v.array(urlSchema),
});

const SCRIPT_FILE_EXTENSIONS = ['.srt', '.vtt', '.txt'];

function emptyForm(defaultCategoryId = ''): NoteFormState {
  return {
    title: '',
    categoryId: defaultCategoryId,
    content: '',
    urlInput: '',
    urls: [],
    noteVideos: [],
    images: [],
  };
}

function extractYoutubeVideoId(url: string) {
  try {
    const parsed = new URL(url);
    const host = parsed.hostname.replace(/^www\./, '').toLowerCase();
    if (host === 'youtu.be') {
      return parsed.pathname.replace(/^\/+/, '').trim() || null;
    }
    if (host === 'youtube.com' || host === 'm.youtube.com') {
      const watchId = parsed.searchParams.get('v');
      if (watchId) {
        return watchId;
      }
      const parts = parsed.pathname.split('/').filter(Boolean);
      if (parts[0] === 'shorts' || parts[0] === 'embed') {
        return parts[1] ?? null;
      }
    }
    return null;
  } catch {
    return null;
  }
}

function syncNoteVideos(urls: string[], existing: NoteVideoRecord[]) {
  const byVideoId = new Map(existing.map((video) => [video.videoId, video]));
  const videos: NoteVideoRecord[] = [];
  for (const url of urls) {
    const videoId = extractYoutubeVideoId(url);
    if (!videoId) {
      continue;
    }
    const current = byVideoId.get(videoId);
    videos.push({
      uid: current?.uid,
      id: current?.id ?? crypto.randomUUID(),
      url,
      videoId,
      scriptContent: current?.scriptContent ?? null,
    });
  }
  return videos;
}

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

function buildTags(title: string, categoryName: string | undefined, verses: NoteVerseInput[]) {
  const slug = title.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/(^-|-$)/g, '');
  return [slug, categoryName, ...verses.map((verse) => verse.fullRef)].filter(Boolean) as string[];
}

function getFirstIssueMessage(result: ReturnType<typeof v.safeParse>) {
  return result.issues?.[0]?.message ?? 'Validation failed.';
}

function closeWindowOrGoBack(navigate: ReturnType<typeof useNavigate>) {
  if (window.opener) {
    window.close();
    return;
  }
  navigate('/notes/list');
}

export function NoteEditorPage() {
  const navigate = useNavigate();
  const { noteId } = useParams();
  const isEditing = Boolean(noteId);
  const [categories, setCategories] = useState<NoteCategory[]>([]);
  const [form, setForm] = useState<NoteFormState>(emptyForm());
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [urlInputError, setUrlInputError] = useState<string | null>(null);
  const [isDragOver, setIsDragOver] = useState(false);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const scriptInputRef = useRef<HTMLInputElement | null>(null);
  const [isScriptDragOver, setIsScriptDragOver] = useState(false);
  const [scriptFileError, setScriptFileError] = useState<string | null>(null);
  const [activeScriptTab, setActiveScriptTab] = useState<string | null>(null);
  const [activeVerse, setActiveVerse] = useState<string | null>(null);
  const [verseLookupError, setVerseLookupError] = useState<string | null>(null);
  const [verseCache, setVerseCache] = useState<VerseCache>({});

  const inferredVerses = useMemo(() => extractVersesFromContent(form.content), [form.content]);
  const youtubeVideos = useMemo(
    () =>
      [...form.noteVideos.map((video) => video.url), ...form.urls]
        .map((url) => {
          const videoId = extractYoutubeVideoId(url);
          return videoId ? { url, videoId } : null;
        })
        .filter((entry, index, list) => entry && list.findIndex((candidate) => candidate?.videoId === entry.videoId) === index)
        .filter((entry): entry is { url: string; videoId: string } => Boolean(entry)),
    [form.noteVideos, form.urls],
  );
  const showScriptField = youtubeVideos.length > 0;
  const editorViewportHeight = 'calc(100vh - 210px)';
  const noteValidation = useMemo(
    () => v.safeParse(noteSchema, { title: form.title, categoryId: form.categoryId, urls: form.urls }),
    [form.title, form.categoryId, form.urls],
  );
  const titleValidation = useMemo(() => v.safeParse(titleSchema, form.title), [form.title]);
  const canSave =
    noteValidation.success &&
    !saving &&
    !form.urlInput.trim() &&
    !form.images.some((image) => image.status === 'uploading');
  const contentInputRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    void (async () => {
      setLoading(true);
      try {
        const loadedCategories = await listCategories();
        setCategories(loadedCategories);
        if (isEditing && noteId) {
          const note = await getNote(noteId);
          if (!note) {
            throw new Error('Note not found.');
          }
          setForm({
            uid: note.uid,
            title: note.title,
            categoryId: note.category?.id ?? '',
            content: note.content,
            urlInput: '',
            urls: (note.urls ?? []).length > 0 ? note.urls : (note.noteVideos ?? []).map((video) => video.url),
            noteVideos: note.noteVideos ?? [],
            images: (note.images ?? []).map((url) => ({
              id: url,
              previewUrl: url,
              uploadedUrl: url,
              status: 'done',
              error: null,
            })),
            createdAt: note.createdAt,
          });
        } else {
          setForm(emptyForm(loadedCategories[0]?.id ?? ''));
        }
        setError(null);
      } catch (loadError) {
        setError(loadError instanceof Error ? loadError.message : 'Failed to load note page.');
      } finally {
        setLoading(false);
      }
    })();
  }, [isEditing, noteId]);

  useEffect(() => {
    if (!showScriptField) {
      setActiveScriptTab(null);
      return;
    }
    const currentStillExists = activeScriptTab && youtubeVideos.some((video) => video.videoId === activeScriptTab);
    if (!currentStillExists) {
      setActiveScriptTab(youtubeVideos[0]?.videoId ?? null);
    }
  }, [activeScriptTab, showScriptField, youtubeVideos]);

  async function handleImageSelection(files: File[] | null) {
    if (!files || files.length === 0) {
      return;
    }
    const staged = files.map((file) => ({
      id: crypto.randomUUID(),
      file,
      previewUrl: URL.createObjectURL(file),
      uploadedUrl: '',
      status: 'uploading' as const,
      error: null,
    }));
    setForm((current) => ({ ...current, images: [...current.images, ...staged] }));
    for (const image of staged) {
      try {
        const uploadedUrl = await uploadImage(image.file!);
        setForm((current) => ({
          ...current,
          images: current.images.map((entry) =>
            entry.id === image.id ? { ...entry, uploadedUrl, previewUrl: uploadedUrl, status: 'done', error: null } : entry,
          ),
        }));
      } catch (uploadError) {
        const message = uploadError instanceof Error ? uploadError.message : 'Upload failed.';
        setForm((current) => ({
          ...current,
          images: current.images.map((entry) => (entry.id === image.id ? { ...entry, status: 'failed', error: message } : entry)),
        }));
      }
    }
  }

  async function retryImage(imageId: string) {
    const target = form.images.find((image) => image.id === imageId);
    if (!target?.file) {
      return;
    }
    setForm((current) => ({
      ...current,
      images: current.images.map((entry) => (entry.id === imageId ? { ...entry, status: 'uploading', error: null } : entry)),
    }));
    try {
      const uploadedUrl = await uploadImage(target.file);
      setForm((current) => ({
        ...current,
        images: current.images.map((entry) =>
          entry.id === imageId ? { ...entry, uploadedUrl, previewUrl: uploadedUrl, status: 'done', error: null } : entry,
        ),
      }));
    } catch (uploadError) {
      const message = uploadError instanceof Error ? uploadError.message : 'Upload failed.';
      setForm((current) => ({
        ...current,
        images: current.images.map((entry) => (entry.id === imageId ? { ...entry, status: 'failed', error: message } : entry)),
      }));
    }
  }

  async function handleScriptSelection(files: File[] | null) {
    if (!files || files.length === 0 || !activeScriptTab) {
      return;
    }

    const file = files[0];
    const lowerName = file.name.toLowerCase();
    const isAllowed = SCRIPT_FILE_EXTENSIONS.some((extension) => lowerName.endsWith(extension));
    if (!isAllowed) {
      setScriptFileError('Only .srt, .vtt, and .txt files are allowed.');
      return;
    }

    try {
      const text = await file.text();
      setForm((current) => ({
        ...current,
        noteVideos: current.noteVideos.map((video) =>
          video.videoId === activeScriptTab ? { ...video, scriptContent: text } : video,
        ),
      }));
      setScriptFileError(null);
    } catch {
      setScriptFileError('Failed to read script file.');
    }
  }

  function addUrl() {
    const nextUrl = form.urlInput.trim();
    if (!nextUrl) {
      return;
    }
    const parsed = v.safeParse(urlSchema, nextUrl);
    if (!parsed.success) {
      setUrlInputError(getFirstIssueMessage(parsed));
      return;
    }
    if (form.urls.some((entry) => entry.toLowerCase() === nextUrl.toLowerCase())) {
      setForm((current) => ({ ...current, urlInput: '' }));
      setUrlInputError(null);
      return;
    }
    setForm((current) => {
      const urls = [...current.urls, nextUrl];
      return { ...current, urls, urlInput: '', noteVideos: syncNoteVideos(urls, current.noteVideos) };
    });
    setUrlInputError(null);
  }

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

  async function handleSave() {
    setError(null);
    setSuccess(null);
    if (form.urlInput.trim()) {
      setError('Add or clear the URL currently in the input before saving.');
      return;
    }
    if (!noteValidation.success) {
      setError(getFirstIssueMessage(noteValidation));
      return;
    }
    if (form.images.some((image) => image.status === 'uploading')) {
      setError('Wait for image uploads to finish before saving.');
      return;
    }
    setSaving(true);
    try {
      const duplicate = await checkDuplicateNoteTitle(form.title.trim(), noteId);
      if (duplicate) {
        throw new Error('A note with this title already exists.');
      }
      const selectedCategory = categories.find((category) => category.id === form.categoryId);
      const now = new Date().toISOString();
      const payload: SaveNoteInput = {
        uid: form.uid,
        title: form.title.trim(),
        content: form.content.trim(),
        urls: form.urls,
        images: form.images.filter((image) => image.status === 'done' && image.uploadedUrl).map((image) => image.uploadedUrl),
        tags: buildTags(form.title, selectedCategory?.name, inferredVerses),
        summary: null,
        createdAt: form.createdAt,
        updatedAt: now,
        categoryId: form.categoryId,
        noteVideos: form.noteVideos.map((video) => ({
          ...video,
          scriptContent: video.scriptContent?.trim() ? video.scriptContent.trim() : null,
        })),
      };
      await saveNote(payload, inferredVerses);
      setSuccess('Note saved successfully.');
      setTimeout(() => navigate('/notes/list'), 400);
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : 'Save failed.');
    } finally {
      setSaving(false);
    }
  }

  function wrapSelection(prefix: string, suffix = prefix) {
    const textarea = contentInputRef.current;
    if (!textarea || loading) {
      return;
    }
    const { selectionStart, selectionEnd, value } = textarea;
    const selected = value.slice(selectionStart, selectionEnd);
    const nextValue = `${value.slice(0, selectionStart)}${prefix}${selected}${suffix}${value.slice(selectionEnd)}`;
    const caretStart = selectionStart + prefix.length;
    const caretEnd = caretStart + selected.length;
    setForm((current) => ({ ...current, content: nextValue }));
    requestAnimationFrame(() => {
      textarea.focus();
      textarea.setSelectionRange(caretStart, caretEnd);
    });
  }

  function insertBlock(prefix: string) {
    const textarea = contentInputRef.current;
    if (!textarea || loading) {
      return;
    }
    const { selectionStart, selectionEnd, value } = textarea;
    const lineStart = value.lastIndexOf('\n', selectionStart - 1) + 1;
    const selected = value.slice(lineStart, selectionEnd);
    const nextValue = `${value.slice(0, lineStart)}${prefix}${selected}${value.slice(selectionEnd)}`;
    setForm((current) => ({ ...current, content: nextValue }));
    requestAnimationFrame(() => {
      textarea.focus();
      const caret = selectionEnd + prefix.length;
      textarea.setSelectionRange(caret, caret);
    });
  }

  return (
    <Stack gap="lg">
      <Group justify="space-between" align="center">
        <Title order={2}>{isEditing ? 'Edit Note' : 'Add Note'}</Title>
        <Group>
          <Button variant="default" onClick={() => closeWindowOrGoBack(navigate)}>
            Cancel
          </Button>
          <Button onClick={() => void handleSave()} disabled={!canSave} loading={saving}>
            Save
          </Button>
        </Group>
      </Group>

      {error && (
        <Alert color="red" icon={<IconAlertCircle size={16} />}>
          {error}
        </Alert>
      )}
      {success && (
        <Alert color="green" icon={<IconCheck size={16} />}>
          {success}
        </Alert>
      )}

      <SimpleGrid cols={{ base: 1, lg: 2 }} spacing="lg">
        <Paper radius="xl" p="lg" withBorder style={{ height: editorViewportHeight, display: 'flex' }}>
          <Stack gap="md" style={{ flex: 1, minHeight: 0 }}>
            <Group grow>
              <TextInput
                label="Title"
                placeholder="Sermon title or study heading"
                value={form.title}
                onChange={(event) => {
                  const value = event.currentTarget.value;
                  setForm((current) => ({ ...current, title: value }));
                }}
                error={form.title && !titleValidation.success ? getFirstIssueMessage(titleValidation) : null}
                disabled={loading}
              />
              <Select
                label="Category"
                placeholder="Choose category"
                data={buildCategoryOptions(categories)}
                value={form.categoryId}
                onChange={(value) => setForm((current) => ({ ...current, categoryId: value ?? '' }))}
                disabled={loading}
              />
            </Group>

            <Box style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
              <Text fw={500} size="sm" mb={6}>
                Content
              </Text>
              <Box
                style={{
                  flex: 1,
                  minHeight: 0,
                  border: '1px solid #ced4da',
                  borderRadius: 12,
                  display: 'flex',
                  flexDirection: 'column',
                  overflow: 'hidden',
                }}
              >
                <Group gap={4} p="xs" style={{ borderBottom: '1px solid #e9ecef' }}>
                  <ActionIcon variant="subtle" color="gray" onClick={() => wrapSelection('**')} disabled={loading} aria-label="Bold">
                    <IconBold size={16} />
                  </ActionIcon>
                  <ActionIcon variant="subtle" color="gray" onClick={() => wrapSelection('*')} disabled={loading} aria-label="Italic">
                    <IconItalic size={16} />
                  </ActionIcon>
                  <ActionIcon variant="subtle" color="gray" onClick={() => insertBlock('# ')} disabled={loading} aria-label="Heading">
                    <IconH1 size={16} />
                  </ActionIcon>
                  <ActionIcon variant="subtle" color="gray" onClick={() => insertBlock('> ')} disabled={loading} aria-label="Quote">
                    <IconQuote size={16} />
                  </ActionIcon>
                  <ActionIcon variant="subtle" color="gray" onClick={() => insertBlock('- ')} disabled={loading} aria-label="Bullet list">
                    <IconList size={16} />
                  </ActionIcon>
                  <ActionIcon variant="subtle" color="gray" onClick={() => insertBlock('1. ')} disabled={loading} aria-label="Numbered list">
                    <IconListNumbers size={16} />
                  </ActionIcon>
                  <ActionIcon variant="subtle" color="gray" onClick={() => wrapSelection('`')} disabled={loading} aria-label="Inline code">
                    <IconCode size={16} />
                  </ActionIcon>
                  <ActionIcon variant="subtle" color="gray" onClick={() => wrapSelection('[', '](https://)')} disabled={loading} aria-label="Link">
                    <IconLink size={16} />
                  </ActionIcon>
                </Group>
                <Textarea
                  ref={contentInputRef}
                  value={form.content}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    setForm((current) => ({ ...current, content: value }));
                  }}
                  placeholder="Write the full markdown body here"
                  disabled={loading}
                  autosize={false}
                  minRows={10}
                  styles={{
                    root: { flex: 1, minHeight: 0 },
                    wrapper: { height: '100%' },
                    input: {
                      height: '100%',
                      minHeight: '100%',
                      border: 0,
                      borderRadius: 0,
                      resize: 'none',
                      padding: '14px 16px',
                      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace',
                      fontSize: 14,
                      lineHeight: 1.6,
                    },
                  }}
                />
              </Box>
            </Box>

            <Stack gap="xs">
              <Text fw={500} size="sm">
                URLs
              </Text>
              <Group align="flex-start">
                <TextInput
                  placeholder="https://youtube.com/watch?v=..."
                  leftSection={<IconLink size={16} />}
                  value={form.urlInput}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    setForm((current) => ({ ...current, urlInput: value }));
                    setUrlInputError(null);
                  }}
                  error={urlInputError}
                  style={{ flex: 1 }}
                />
                <Button leftSection={<IconPlus size={16} />} onClick={addUrl}>
                  Add
                </Button>
              </Group>
              <Group gap="xs">
                {form.urls.length === 0 ? <Text c="dimmed">No URLs added yet.</Text> : null}
                {form.urls.map((url) => (
                  <Badge
                    key={url}
                    variant="light"
                    rightSection={
                      <ActionIcon
                        size="xs"
                        variant="transparent"
                        onClick={() =>
                          setForm((current) => {
                            const urls = current.urls.filter((entry) => entry !== url);
                            return { ...current, urls, noteVideos: syncNoteVideos(urls, current.noteVideos) };
                          })
                        }
                      >
                        <IconTrash size={12} />
                      </ActionIcon>
                    }
                  >
                    {url}
                  </Badge>
                ))}
              </Group>
            </Stack>
          </Stack>
        </Paper>

        <Stack gap="lg" style={{ minHeight: editorViewportHeight }}>
          <Paper radius="xl" p="lg" withBorder>
            <Stack gap="md">
              <Group justify="space-between">
                <Group gap="sm">
                  <ThemeIcon variant="light" color="orange" radius="xl">
                    <IconPhoto size={16} />
                  </ThemeIcon>
                  <Title order={4}>Image Upload</Title>
                </Group>
                <Text c="dimmed" size="sm">
                  Uploads start immediately
                </Text>
              </Group>
              <Box
                onClick={() => fileInputRef.current?.click()}
                onDragEnter={(event) => {
                  event.preventDefault();
                  setIsDragOver(true);
                }}
                onDragOver={(event) => {
                  event.preventDefault();
                  setIsDragOver(true);
                }}
                onDragLeave={(event) => {
                  event.preventDefault();
                  if (event.currentTarget.contains(event.relatedTarget as Node | null)) {
                    return;
                  }
                  setIsDragOver(false);
                }}
                onDrop={(event) => {
                  event.preventDefault();
                  setIsDragOver(false);
                  void handleImageSelection(Array.from(event.dataTransfer.files));
                }}
                style={{
                  border: `2px dashed ${isDragOver ? '#228be6' : '#cfd8dc'}`,
                  borderRadius: 20,
                  padding: 36,
                  textAlign: 'center',
                  background: isDragOver ? '#eef6ff' : '#fafbfc',
                  cursor: 'pointer',
                }}
              >
                <ThemeIcon variant="light" color="orange" radius="xl" size="xl" mx="auto" mb="md">
                  <IconUpload size={20} />
                </ThemeIcon>
                <Text fw={600}>Drag images here</Text>
                <Text c="dimmed">or click to choose files</Text>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept="image/*"
                  multiple
                  hidden
                  onChange={(event) => {
                    void handleImageSelection(Array.from(event.currentTarget.files ?? []));
                    event.currentTarget.value = '';
                  }}
                />
              </Box>
              <Stack gap="sm">
                {form.images.map((image) => (
                  <Card key={image.id} withBorder radius="lg" padding="sm">
                    <Group align="flex-start" wrap="nowrap">
                      <Image src={image.previewUrl} alt="" w={92} h={92} radius="md" fit="cover" />
                      <Stack gap={4} style={{ flex: 1 }}>
                        <Text size="sm" fw={600}>
                          {image.file?.name ?? image.uploadedUrl.split('/').pop() ?? 'Uploaded image'}
                        </Text>
                        <Badge color={image.status === 'failed' ? 'red' : image.status === 'done' ? 'green' : 'blue'} variant="light">
                          {image.status}
                        </Badge>
                        {image.error ? (
                          <Text size="xs" c="red">
                            {image.error}
                          </Text>
                        ) : null}
                        <Group gap="xs">
                          {image.status === 'failed' && image.file ? (
                            <Button size="xs" variant="light" leftSection={<IconRefresh size={14} />} onClick={() => void retryImage(image.id)}>
                              Retry
                            </Button>
                          ) : null}
                          <Button size="xs" variant="subtle" color="red" leftSection={<IconTrash size={14} />} onClick={() => setForm((current) => ({ ...current, images: current.images.filter((entry) => entry.id !== image.id) }))}>
                            Remove
                          </Button>
                        </Group>
                      </Stack>
                    </Group>
                  </Card>
                ))}
              </Stack>
            </Stack>
          </Paper>

          {showScriptField ? (
            <Paper radius="xl" p="lg" withBorder>
              <Stack gap="md">
                <Group justify="space-between">
                  <Title order={4}>Script / Subtitles</Title>
                  <Text c="dimmed" size="sm">
                    One tab per YouTube video
                  </Text>
                </Group>
                <Tabs value={activeScriptTab} onChange={setActiveScriptTab}>
                  <Tabs.List>
                    {youtubeVideos.map((video) => (
                      <Tabs.Tab key={video.videoId} value={video.videoId}>
                        {video.videoId}
                      </Tabs.Tab>
                    ))}
                  </Tabs.List>

                  {youtubeVideos.map((video) => (
                    <Tabs.Panel key={video.videoId} value={video.videoId} pt="md">
                      <Stack gap="xs">
                        <Box
                          onClick={() => scriptInputRef.current?.click()}
                          onDragEnter={(event) => {
                            event.preventDefault();
                            setIsScriptDragOver(true);
                            setActiveScriptTab(video.videoId);
                          }}
                          onDragOver={(event) => {
                            event.preventDefault();
                            setIsScriptDragOver(true);
                            setActiveScriptTab(video.videoId);
                          }}
                          onDragLeave={(event) => {
                            event.preventDefault();
                            if (event.currentTarget.contains(event.relatedTarget as Node | null)) {
                              return;
                            }
                            setIsScriptDragOver(false);
                          }}
                          onDrop={(event) => {
                            event.preventDefault();
                            setIsScriptDragOver(false);
                            setActiveScriptTab(video.videoId);
                            void handleScriptSelection(Array.from(event.dataTransfer.files));
                          }}
                          style={{
                            border: `2px dashed ${isScriptDragOver && activeScriptTab === video.videoId ? '#228be6' : '#cfd8dc'}`,
                            borderRadius: 16,
                            padding: 20,
                            textAlign: 'center',
                            background: isScriptDragOver && activeScriptTab === video.videoId ? '#eef6ff' : '#fafbfc',
                            cursor: 'pointer',
                          }}
                        >
                          <ThemeIcon variant="light" color="blue" radius="xl" size="lg" mx="auto" mb="sm">
                            <IconUpload size={18} />
                          </ThemeIcon>
                          <Text fw={600}>Drop subtitle files here</Text>
                          <Text c="dimmed" size="sm">
                            Only .srt, .vtt, and .txt are allowed
                          </Text>
                          <input
                            ref={scriptInputRef}
                            type="file"
                            accept=".srt,.vtt,.txt,text/plain"
                            hidden
                            onChange={(event) => {
                              void handleScriptSelection(Array.from(event.currentTarget.files ?? []));
                              event.currentTarget.value = '';
                            }}
                          />
                        </Box>
                        {scriptFileError && activeScriptTab === video.videoId ? <Text c="red" size="sm">{scriptFileError}</Text> : null}
                        <Textarea
                          label="Transcript"
                          rows={10}
                          placeholder="Paste transcript or subtitle text"
                          value={form.noteVideos.find((entry) => entry.videoId === video.videoId)?.scriptContent ?? ''}
                          onChange={(event) => {
                            const value = event.currentTarget.value;
                            setForm((current) => ({
                              ...current,
                              noteVideos: current.noteVideos.map((entry) =>
                                entry.videoId === video.videoId ? { ...entry, scriptContent: value } : entry,
                              ),
                            }));
                          }}
                        />
                      </Stack>
                    </Tabs.Panel>
                  ))}
                </Tabs>
              </Stack>
            </Paper>
          ) : null}

          <Paper radius="xl" p="lg" withBorder style={{ flex: 1, minHeight: 0, display: 'flex' }}>
            <Stack gap="md" style={{ flex: 1, minHeight: 0 }}>
              <Group justify="space-between">
                <Title order={4}>Detected Verses</Title>
                <Text c="dimmed" size="sm">
                  Inferred from content
                </Text>
              </Group>
              <Box style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}>
                {inferredVerses.length === 0 ? <Text c="dimmed">No verse references detected in the content yet.</Text> : null}
                <Group gap="xs">
                  {inferredVerses.map((verse) => (
                    <Popover key={verse.id} width={360} position="bottom-start" opened={activeVerse === verse.fullRef} onChange={(opened) => setActiveVerse(opened ? verse.fullRef : null)}>
                      <Popover.Target>
                        <Badge variant="light" style={{ cursor: 'pointer' }} onClick={() => void openVerse(verse.fullRef)}>
                          {verse.fullRef}
                        </Badge>
                      </Popover.Target>
                      <Popover.Dropdown>
                        <Stack gap="xs">
                          <Text fw={600}>{verse.fullRef}</Text>
                          {verseLookupError && activeVerse === verse.fullRef ? <Text c="red" size="sm">{verseLookupError}</Text> : null}
                          {(verseCache[verse.fullRef] ?? []).map((record) => (
                            <Box key={`${verse.fullRef}-${record.verseNumber}`}>
                              <Text size="sm" fw={600}>
                                {record.verseNumber}
                              </Text>
                              <Text size="sm">{record.text}</Text>
                            </Box>
                          ))}
                          {!verseLookupError && !verseCache[verse.fullRef] ? <Text c="dimmed" size="sm">Loading verse text...</Text> : null}
                        </Stack>
                      </Popover.Dropdown>
                    </Popover>
                  ))}
                </Group>
              </Box>
            </Stack>
          </Paper>
        </Stack>
      </SimpleGrid>
    </Stack>
  );
}
