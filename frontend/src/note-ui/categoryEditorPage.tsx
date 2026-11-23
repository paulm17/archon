import { useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { Alert, Button, Group, Paper, Select, Stack, TextInput, Title } from '@mantine/core';
import { IconAlertCircle, IconArrowLeft, IconCheck } from '@tabler/icons-react';
import * as v from 'valibot';
import { createCategory, getCategory, listCategories, updateCategory, type NoteCategory } from './api';
import { buildCategoryOptions, wouldCreateCategoryCycle } from './categoryTree';

const categorySchema = v.pipe(v.string(), v.trim(), v.minLength(1, 'Category name is required.'));

export function CategoryEditorPage() {
  const navigate = useNavigate();
  const { categoryId } = useParams();
  const isEditing = Boolean(categoryId);
  const [name, setName] = useState('');
  const [parentId, setParentId] = useState<string | null>(null);
  const [categories, setCategories] = useState<NoteCategory[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const validation = v.safeParse(categorySchema, name);

  useEffect(() => {
    if (!isEditing || !categoryId) {
      return;
    }
    void (async () => {
      setLoading(true);
      try {
        const allCategories = await listCategories();
        setCategories(allCategories);
        const category = await getCategory(categoryId);
        if (!category) {
          throw new Error('Category not found.');
        }
        setName(category.name);
        setParentId(category.parent?.id ?? null);
      } catch (loadError) {
        setError(loadError instanceof Error ? loadError.message : 'Failed to load category.');
      } finally {
        setLoading(false);
      }
    })();
  }, [categoryId, isEditing]);

  useEffect(() => {
    if (isEditing) {
      return;
    }

    void (async () => {
      setLoading(true);
      try {
        setCategories(await listCategories());
      } catch (loadError) {
        setError(loadError instanceof Error ? loadError.message : 'Failed to load categories.');
      } finally {
        setLoading(false);
      }
    })();
  }, [isEditing]);

  async function handleSave() {
    if (!validation.success) {
      setError(validation.issues?.[0]?.message ?? 'Validation failed.');
      return;
    }
    if (isEditing && categoryId && wouldCreateCategoryCycle(categories, categoryId, parentId)) {
      setError('This parent category would create a cycle.');
      return;
    }
    setSaving(true);
    setError(null);
    setSuccess(null);
    try {
      if (isEditing && categoryId) {
        await updateCategory(categoryId, name.trim(), parentId);
      } else {
        await createCategory(name.trim(), parentId);
      }
      setSuccess('Category saved successfully.');
      setTimeout(() => navigate('/notes/categories'), 400);
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : 'Failed to save category.');
    } finally {
      setSaving(false);
    }
  }

  return (
    <Stack gap="lg">
      <Group justify="space-between" align="center">
        <Group gap="sm">
          <Button variant="subtle" leftSection={<IconArrowLeft size={16} />} onClick={() => navigate('/notes/categories')}>
            Back to Categories
          </Button>
          <Title order={2}>{isEditing ? 'Edit Category' : 'Add Category'}</Title>
        </Group>
        <Group>
          <Button variant="default" onClick={() => navigate('/notes/categories')}>
            Cancel
          </Button>
          <Button onClick={() => void handleSave()} loading={saving} disabled={!validation.success || loading}>
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

      <Paper radius="xl" p="lg" withBorder>
        <Stack>
          <TextInput
            label="Category name"
            placeholder="ideas"
            value={name}
            onChange={(event) => setName(event.currentTarget.value)}
            error={name && !validation.success ? validation.issues?.[0]?.message : null}
            disabled={loading}
          />
          <Select
            label="Parent category"
            placeholder="Top level"
            clearable
            data={buildCategoryOptions(categories.filter((category) => category.id !== categoryId))}
            value={parentId}
            onChange={setParentId}
            disabled={loading}
            error={isEditing && categoryId && wouldCreateCategoryCycle(categories, categoryId, parentId) ? 'This parent would create a cycle.' : null}
          />
        </Stack>
      </Paper>
    </Stack>
  );
}
