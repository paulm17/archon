import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { ActionIcon, Alert, Badge, Box, Button, Collapse, Group, Paper, SegmentedControl, Select, Stack, Text, TextInput, ThemeIcon, Title } from '@mantine/core';
import { DataTable } from 'mantine-datatable';
import { IconAlertCircle, IconChevronDown, IconDatabaseOff, IconEdit, IconFilter, IconFolderCog, IconPlus, IconSearch, IconSparkles, IconTrash } from '@tabler/icons-react';
import { client } from '../lib/client';
import { openChildWindow } from '../lib/nativeWindow';
import { pipe, subscribe } from 'wonka';
import { deleteNote, listCategories, listNotes, type NoteCategory, type NoteRecord } from './api';
import { buildCategoryOptions } from './categoryTree';
import { NotesWorkspace } from './notesWorkspace';

const NOTES_MAIN_MODE_KEY = 'notes-main-mode';

function readNotesMainMode(): 'list' | 'workspace' {
  if (typeof window === 'undefined') {
    return 'list';
  }

  try {
    const stored = window.localStorage?.getItem(NOTES_MAIN_MODE_KEY);
    return stored === 'workspace' ? 'workspace' : 'list';
  } catch {
    return 'list';
  }
}

function writeNotesMainMode(mode: 'list' | 'workspace') {
  if (typeof window === 'undefined') {
    return;
  }

  try {
    window.localStorage?.setItem(NOTES_MAIN_MODE_KEY, mode);
  } catch {
    // Ignore storage failures in embedded runtimes where localStorage is unavailable.
  }
}

export function NotesListPage() {
  const navigate = useNavigate();
  const [mode, setMode] = useState<'list' | 'workspace'>(() => readNotesMainMode());
  const [notes, setNotes] = useState<NoteRecord[]>([]);
  const [categories, setCategories] = useState<NoteCategory[]>([]);
  const [totalRecords, setTotalRecords] = useState(0);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(10);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [titleSearch, setTitleSearch] = useState('');
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [fullTextSearch, setFullTextSearch] = useState('');
  const [filterCategoryId, setFilterCategoryId] = useState<string | null>(null);
  const [createdAfter, setCreatedAfter] = useState('');
  const [updatedAfter, setUpdatedAfter] = useState('');
  const [sortField, setSortField] = useState<'createdAt' | 'updatedAt' | 'title'>('updatedAt');
  const [sortDirection, setSortDirection] = useState<'ASC' | 'DESC'>('DESC');
  const [refreshTick, setRefreshTick] = useState(0);

  useEffect(() => {
    void (async () => {
      try {
        setCategories(await listCategories());
      } catch (loadError) {
        setError(loadError instanceof Error ? loadError.message : 'Failed to load categories.');
      }
    })();
  }, []);

  useEffect(() => {
    writeNotesMainMode(mode);
  }, [mode]);

  useEffect(() => {
    const subscription = pipe(
      client.subscription(
        `
          subscription NotesEvents($types: [String]) {
            event(types: $types) {
              type
              uid
              mutation
            }
          }
        `,
        { types: ['Note', 'NoteCategory', 'NoteVerse', 'NoteRelationship'] },
      ),
      subscribe(() => {
        setRefreshTick((current) => current + 1);
      }),
    );

    return () => {
      subscription.unsubscribe();
    };
  }, []);

  useEffect(() => {
    if (mode !== 'list') {
      return;
    }

    void (async () => {
      setLoading(true);
      try {
        const result = await listNotes(page, pageSize, {
          titleSearch,
          fullTextSearch,
          categoryId: filterCategoryId ?? undefined,
          createdAfter: createdAfter ? new Date(`${createdAfter}T00:00:00`).toISOString() : undefined,
          updatedAfter: updatedAfter ? new Date(`${updatedAfter}T00:00:00`).toISOString() : undefined,
          sortField,
          sortDirection,
        });
        setNotes(result.records);
        setTotalRecords(result.total);
        setError(null);
      } catch (loadError) {
        setError(loadError instanceof Error ? loadError.message : 'Failed to load notes.');
      } finally {
        setLoading(false);
      }
    })();
  }, [mode, page, pageSize, titleSearch, fullTextSearch, filterCategoryId, createdAfter, updatedAfter, sortField, sortDirection, refreshTick]);

  async function handleDeleteNote(noteId: string) {
    if (!window.confirm('Delete this note?')) {
      return;
    }
    try {
      await deleteNote(noteId);
      const result = await listNotes(page, pageSize, {
        titleSearch,
        fullTextSearch,
        categoryId: filterCategoryId ?? undefined,
        createdAfter: createdAfter ? new Date(`${createdAfter}T00:00:00`).toISOString() : undefined,
        updatedAfter: updatedAfter ? new Date(`${updatedAfter}T00:00:00`).toISOString() : undefined,
        sortField,
        sortDirection,
      });
      setNotes(result.records);
      setTotalRecords(result.total);
    } catch (deleteError) {
      setError(deleteError instanceof Error ? deleteError.message : 'Failed to delete note.');
    }
  }

  async function openViewWindow(noteId: string) {
    await openChildWindow({
      title: 'View Note',
      url: `${window.location.origin}/notes/${noteId}/view?child=1`,
      width: 1200,
      height: 900,
      persist_key: 'notes-view',
    });
  }

  async function openEditWindow(noteId: string) {
    await openChildWindow({
      title: 'Edit Note',
      url: `${window.location.origin}/notes/${noteId}/edit?child=1`,
      width: 1280,
      height: 920,
      persist_key: 'notes-edit',
    });
  }

  async function openAddWindow() {
    await openChildWindow({
      title: 'Add Note',
      url: `${window.location.origin}/notes/new?child=1`,
      width: 1280,
      height: 920,
      persist_key: 'notes-add',
    });
  }

  return (
    <Stack gap="lg">
      <Paper
        radius="xl"
        p="xl"
        style={{
          background: 'linear-gradient(135deg, #f4efe7 0%, #e6f1ea 100%)',
          border: '1px solid #d7e1da',
        }}
      >
        <Group justify="space-between" align="center">
          <Box>
            <Group gap="sm" mb="xs">
              <ThemeIcon variant="light" color="teal" radius="xl" size="lg">
                <IconSparkles size={18} />
              </ThemeIcon>
              <Title order={2}>Notes</Title>
            </Group>
            <Text c="dimmed">
              {mode === 'workspace'
                ? 'Explore your notes in a split workspace with a tree and tabbed reader.'
                : 'Browse, search, and organise notes by category.'}
            </Text>
          </Box>
          <Group>
            <SegmentedControl
              value={mode}
              onChange={(value) => setMode(value as 'list' | 'workspace')}
              data={[
                { label: 'List', value: 'list' },
                { label: 'Workspace', value: 'workspace' },
              ]}
            />
            <Button leftSection={<IconFolderCog size={16} />} variant="light" onClick={() => navigate('/notes/categories')}>
              Manage Categories
            </Button>
            <Button leftSection={<IconPlus size={16} />} onClick={() => void openAddWindow()}>
              Add Note
            </Button>
          </Group>
        </Group>
      </Paper>

      {error && (
        <Alert color="red" icon={<IconAlertCircle size={16} />}>
          {error}
        </Alert>
      )}

      {mode === 'workspace' ? (
        <NotesWorkspace />
      ) : (
        <>
          <Paper radius="xl" p="lg" withBorder>
            <Stack gap="md">
              <Group justify="space-between" align="flex-end">
                <TextInput
                  label="Search title"
                  placeholder="Search by title"
                  leftSection={<IconSearch size={16} />}
                  value={titleSearch}
                  onChange={(event) => {
                    setPage(1);
                    setTitleSearch(event.currentTarget.value);
                  }}
                  style={{ flex: 1 }}
                />
                <Button
                  variant="subtle"
                  leftSection={<IconFilter size={16} />}
                  rightSection={<IconChevronDown size={16} />}
                  onClick={() => setAdvancedOpen((current) => !current)}
                >
                  Advanced Search
                </Button>
              </Group>

              <Collapse in={advancedOpen}>
                <Group grow align="flex-end">
                  <TextInput
                    label="Title or content"
                    placeholder="Full-text search"
                    value={fullTextSearch}
                    onChange={(event) => {
                      setPage(1);
                      setFullTextSearch(event.currentTarget.value);
                    }}
                  />
                  <Select
                    label="Category"
                    placeholder="All categories"
                    clearable
                    data={buildCategoryOptions(categories)}
                    value={filterCategoryId}
                    onChange={(value) => {
                      setPage(1);
                      setFilterCategoryId(value);
                    }}
                  />
                  <Select
                    label="Sort"
                    data={[
                      { value: 'updatedAt:DESC', label: 'Last updated (newest)' },
                      { value: 'updatedAt:ASC', label: 'Last updated (oldest)' },
                      { value: 'createdAt:DESC', label: 'Created (newest)' },
                      { value: 'createdAt:ASC', label: 'Created (oldest)' },
                      { value: 'title:ASC', label: 'Title (A-Z)' },
                      { value: 'title:DESC', label: 'Title (Z-A)' },
                    ]}
                    value={`${sortField}:${sortDirection}`}
                    onChange={(value) => {
                      if (!value) {
                        return;
                      }
                      const [field, direction] = value.split(':') as ['createdAt' | 'updatedAt' | 'title', 'ASC' | 'DESC'];
                      setSortField(field);
                      setSortDirection(direction);
                      setPage(1);
                    }}
                  />
                  <TextInput
                    label="Created after"
                    type="date"
                    value={createdAfter}
                    onChange={(event) => {
                      setPage(1);
                      setCreatedAfter(event.currentTarget.value);
                    }}
                  />
                  <TextInput
                    label="Updated after"
                    type="date"
                    value={updatedAfter}
                    onChange={(event) => {
                      setPage(1);
                      setUpdatedAfter(event.currentTarget.value);
                    }}
                  />
                </Group>
              </Collapse>
            </Stack>
          </Paper>

          <Paper radius="xl" p="md" withBorder>
            <Stack gap="md">
              <Group justify="flex-end" gap="sm" align="center">
                <Text size="sm">Records per page</Text>
                <Select
                  data={[
                    { value: '10', label: '10' },
                    { value: '20', label: '20' },
                    { value: '50', label: '50' },
                  ]}
                  value={String(pageSize)}
                  onChange={(value) => {
                    if (!value) {
                      return;
                    }
                    setPageSize(Number(value));
                    setPage(1);
                  }}
                  w={110}
                />
              </Group>

              {notes.length === 0 && !loading ? (
                <Box py="xl" style={{ display: 'flex', justifyContent: 'center', alignItems: 'center', minHeight: 260 }}>
                  <Group gap={6} c="dimmed">
                    <IconDatabaseOff size={18} />
                    <Text size="sm">No records</Text>
                  </Group>
                </Box>
              ) : (
                <DataTable
                  withTableBorder
                  borderRadius="md"
                  striped
                  highlightOnHover
                  fetching={loading}
                  records={notes}
                  totalRecords={totalRecords}
                  recordsPerPage={pageSize}
                  page={page}
                  onPageChange={setPage}
                  paginationText={({ from, to, totalRecords }) => `${from} - ${to} / ${totalRecords}`}
                  renderPagination={({ Controls }) => (
                    <Group justify="space-between" mt="md" px="xs">
                      <Controls.Text />
                      <Controls.Pagination />
                    </Group>
                  )}
                  columns={[
                    {
                      accessor: 'title',
                      title: 'Title',
                      width: '40%',
                      render: (record) => (
                        <Text
                          component="button"
                          type="button"
                          onClick={() => void openViewWindow(record.id)}
                          style={{
                            background: 'none',
                            border: 0,
                            padding: 0,
                            font: 'inherit',
                            color: 'inherit',
                            cursor: 'pointer',
                            textAlign: 'left',
                          }}
                        >
                          {record.title}
                        </Text>
                      ),
                    },
                    {
                      accessor: 'category',
                      title: 'Category',
                      render: (record) => <Badge variant="light">{record.category?.name ?? 'root'}</Badge>,
                    },
                    {
                      accessor: 'createdAt',
                      title: 'Created',
                      render: (record) => new Date(record.createdAt).toLocaleDateString(),
                    },
                    {
                      accessor: 'updatedAt',
                      title: 'Updated',
                      render: (record) => new Date(record.updatedAt).toLocaleDateString(),
                    },
                    {
                      accessor: 'actions',
                      title: '',
                      width: 110,
                      render: (record) => (
                        <Group gap={4} wrap="nowrap">
                          <ActionIcon variant="subtle" color="blue" onClick={() => void openEditWindow(record.id)}>
                            <IconEdit size={16} />
                          </ActionIcon>
                          <ActionIcon variant="subtle" color="red" onClick={() => void handleDeleteNote(record.id)}>
                            <IconTrash size={16} />
                          </ActionIcon>
                        </Group>
                      ),
                    },
                  ]}
                  minHeight={260}
                />
              )}
            </Stack>
          </Paper>
        </>
      )}
    </Stack>
  );
}
