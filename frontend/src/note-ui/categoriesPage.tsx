import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { ActionIcon, Alert, Button, Group, Paper, Stack, Text, ThemeIcon, Title } from '@mantine/core';
import { DataTable } from 'mantine-datatable';
import { IconAlertCircle, IconEdit, IconFolderCog, IconPlus, IconTrash, IconX } from '@tabler/icons-react';
import { client } from '../lib/client';
import { pipe, subscribe } from 'wonka';
import { countChildCategories, countNotesForCategory, deleteCategory, listCategories, type NoteCategory } from './api';
import { buildCategoryPathMap } from './categoryTree';

export function CategoriesPage() {
  const navigate = useNavigate();
  const [categories, setCategories] = useState<NoteCategory[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const pathMap = buildCategoryPathMap(categories);

  async function refresh() {
    setLoading(true);
    try {
      setCategories(await listCategories());
      setError(null);
    } catch (loadError) {
      setError(loadError instanceof Error ? loadError.message : 'Failed to load categories.');
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
  }, [refreshTick]);

  useEffect(() => {
    const subscription = pipe(
      client.subscription(
        `
          subscription CategoryEvents($types: [String]) {
            event(types: $types) {
              type
              uid
              mutation
            }
          }
        `,
        { types: ['NoteCategory', 'Note'] },
      ),
      subscribe(() => {
        setRefreshTick((current) => current + 1);
      }),
    );

    return () => {
      subscription.unsubscribe();
    };
  }, []);

  async function handleDelete(category: NoteCategory) {
    if (!window.confirm(`Delete category "${category.name}"?`)) {
      return;
    }
    try {
      const childCount = await countChildCategories(category.id);
      if (childCount > 0) {
        throw new Error(`Cannot delete category "${category.name}" while ${childCount} child categor${childCount === 1 ? 'y' : 'ies'} exist.`);
      }
      const noteCount = await countNotesForCategory(category.id);
      if (noteCount > 0) {
        throw new Error(`Cannot delete category "${category.name}" while ${noteCount} note(s) are assigned to it.`);
      }
      await deleteCategory(category.id);
      await refresh();
    } catch (deleteError) {
      setError(deleteError instanceof Error ? deleteError.message : 'Failed to delete category.');
    }
  }

  return (
    <Stack gap="lg">
      <Group justify="space-between" align="center">
        <Group gap="sm">
          <ThemeIcon variant="light" color="teal" radius="xl" size="lg">
            <IconFolderCog size={18} />
          </ThemeIcon>
          <Title order={2}>Manage Categories</Title>
        </Group>
        <Group>
          <Button leftSection={<IconPlus size={16} />} onClick={() => navigate('/notes/categories/new')}>
            Add Category
          </Button>
          <Button variant="default" leftSection={<IconX size={16} />} onClick={() => navigate('/notes/list')}>
            Close
          </Button>
        </Group>
      </Group>

      <Text c="dimmed">View, edit, and delete note categories.</Text>

      {error && (
        <Alert color="red" icon={<IconAlertCircle size={16} />}>
          {error}
        </Alert>
      )}

      <Paper radius="xl" p="md" withBorder>
        <DataTable
          withTableBorder
          borderRadius="md"
          striped
          highlightOnHover
          fetching={loading}
          records={categories}
          columns={[
            {
              accessor: 'name',
              title: 'Name',
              render: (record) => pathMap.get(record.id) ?? record.name,
            },
            {
              accessor: 'parent',
              title: 'Parent',
              render: (record) => record.parent?.name ?? 'Top level',
            },
            {
              accessor: 'createdAt',
              title: 'Created',
              render: (record) => new Date(record.createdAt).toLocaleDateString(),
            },
            {
              accessor: 'actions',
              title: '',
              width: 110,
              render: (record) => (
                <Group gap={4} wrap="nowrap">
                  <ActionIcon variant="subtle" color="blue" onClick={() => navigate(`/notes/categories/${record.id}/edit`)}>
                    <IconEdit size={16} />
                  </ActionIcon>
                  <ActionIcon variant="subtle" color="red" onClick={() => void handleDelete(record)}>
                    <IconTrash size={16} />
                  </ActionIcon>
                </Group>
              ),
            },
          ]}
          minHeight={260}
        />
      </Paper>
    </Stack>
  );
}
