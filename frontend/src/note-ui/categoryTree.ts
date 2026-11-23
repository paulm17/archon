import type { NoteCategory } from './api';

export type CategoryOption = {
  value: string;
  label: string;
};

export function buildCategoryChildrenMap(categories: NoteCategory[]) {
  const children = new Map<string | null, NoteCategory[]>();

  for (const category of categories) {
    const parentId = category.parent?.id ?? null;
    const bucket = children.get(parentId) ?? [];
    bucket.push(category);
    children.set(parentId, bucket);
  }

  for (const bucket of children.values()) {
    bucket.sort((left, right) => left.name.localeCompare(right.name));
  }

  return children;
}

export function buildCategoryPathMap(categories: NoteCategory[]) {
  const byId = new Map(categories.map((category) => [category.id, category]));
  const cache = new Map<string, string>();

  function resolve(categoryId: string, trail = new Set<string>()): string {
    if (cache.has(categoryId)) {
      return cache.get(categoryId)!;
    }

    const category = byId.get(categoryId);
    if (!category) {
      return '';
    }

    if (trail.has(categoryId)) {
      return category.name;
    }

    trail.add(categoryId);
    const parentPath = category.parent?.id ? resolve(category.parent.id, trail) : '';
    trail.delete(categoryId);
    const path = parentPath ? `${parentPath} / ${category.name}` : category.name;
    cache.set(categoryId, path);
    return path;
  }

  for (const category of categories) {
    resolve(category.id);
  }

  return cache;
}

export function buildCategoryOptions(categories: NoteCategory[]): CategoryOption[] {
  const pathMap = buildCategoryPathMap(categories);

  return categories
    .map((category) => ({
      value: category.id,
      label: pathMap.get(category.id) ?? category.name,
    }))
    .sort((left, right) => left.label.localeCompare(right.label));
}

export function wouldCreateCategoryCycle(categories: NoteCategory[], categoryId: string, nextParentId: string | null) {
  if (!nextParentId) {
    return false;
  }

  if (categoryId === nextParentId) {
    return true;
  }

  const byId = new Map(categories.map((category) => [category.id, category]));
  const seen = new Set<string>();
  let currentId: string | null = nextParentId;

  while (currentId) {
    if (currentId === categoryId) {
      return true;
    }

    if (seen.has(currentId)) {
      return true;
    }

    seen.add(currentId);
    currentId = byId.get(currentId)?.parent?.id ?? null;
  }

  return false;
}
