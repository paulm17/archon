import { client } from '../lib/client';

const GRAPHQL_URL = 'http://localhost:8000/graphql';
const VARDA_BASE_URL = GRAPHQL_URL.replace(/\/graphql$/, '');

export type NoteCategory = {
  uid: string;
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  parent?: {
    id: string;
    name: string;
  } | null;
};

export type NotesTreeNote = Pick<NoteRecord, 'id' | 'title' | 'summary' | 'updatedAt'> & {
  category: {
    id: string;
  } | null;
};

export type NotesTreeData = {
  categories: NoteCategory[];
  notes: NotesTreeNote[];
};

export type NoteVerseInput = {
  id: string;
  gospel: string;
  reference: string;
  fullRef: string;
};

export type NoteVideoRecord = {
  uid?: string;
  id: string;
  url: string;
  videoId: string;
  scriptContent: string | null;
};

export type RelatedNoteRecord = {
  id: string;
  title: string;
  summary: string | null;
  relationshipType: string;
  direction: 'incoming' | 'outgoing';
};

export type NoteRecord = {
  uid: string;
  id: string;
  title: string;
  content: string;
  urls: string[];
  images: string[];
  tags: string[];
  summary: string | null;
  createdAt: string;
  updatedAt: string;
  category: NoteCategory | null;
  noteVerses: NoteVerseInput[];
  noteVideos: NoteVideoRecord[];
};

export type SaveNoteInput = {
  uid?: string;
  title: string;
  content: string;
  urls: string[];
  images: string[];
  tags: string[];
  summary: string | null;
  createdAt?: string;
  updatedAt: string;
  categoryId: string;
  noteVideos: NoteVideoRecord[];
};

export type NoteListFilters = {
  titleSearch?: string;
  fullTextSearch?: string;
  categoryId?: string;
  createdAfter?: string;
  updatedAfter?: string;
  sortField?: 'createdAt' | 'updatedAt' | 'title';
  sortDirection?: 'ASC' | 'DESC';
};

export type PaginatedNotes = {
  records: NoteRecord[];
  total: number;
};

export type VerseLookupResult = {
  verseNumber: number | null;
  text: string | null;
  chapterHeading: string | null;
  sectionHeading: string | null;
};

function buildCategoryFields(includeParent = true) {
  return `
    uid
    id
    name
    createdAt
    updatedAt
    ${includeParent ? 'parent { id name }' : ''}
  `;
}

function gqlString(value: unknown): string {
  if (value === null) return 'null';
  if (typeof value === 'string') return JSON.stringify(value);
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) return `[${value.map(gqlString).join(', ')}]`;
  if (typeof value === 'object') {
    return `{${Object.entries(value as Record<string, unknown>)
      .map(([key, entry]) => `${key}: ${gqlString(entry)}`)
      .join(', ')}}`;
  }
  return 'null';
}

function buildNoteFilter(filters: NoteListFilters): string | null {
  const clauses: string[] = [];

  if (filters.titleSearch?.trim()) {
    clauses.push(`{ title: { anyoftext: ${gqlString(filters.titleSearch.trim())} } }`);
  }

  if (filters.fullTextSearch?.trim()) {
    const query = filters.fullTextSearch.trim();
    clauses.push(
      `{ or: [
        { title: { anyoftext: ${gqlString(query)} } },
        { content: { anyoftext: ${gqlString(query)} } }
      ] }`,
    );
  }

  if (filters.categoryId) {
    clauses.push(`{ category: { id: { eq: ${gqlString(filters.categoryId)} } } }`);
  }

  if (filters.createdAfter) {
    clauses.push(`{ createdAt: { ge: ${gqlString(filters.createdAfter)} } }`);
  }

  if (filters.updatedAfter) {
    clauses.push(`{ updatedAt: { ge: ${gqlString(filters.updatedAfter)} } }`);
  }

  if (clauses.length === 0) {
    return null;
  }

  if (clauses.length === 1) {
    return clauses[0].slice(1, -1);
  }

  return `and: [${clauses.join(', ')}]`;
}

function buildNoteFields(includeContent = false) {
  return `
    uid
    id
    title
    ${includeContent ? 'content' : ''}
    summary
    createdAt
    updatedAt
    category {
      id
      name
    }
    noteVerses {
      id
      gospel
      reference
      fullRef
    }
    noteVideos {
      uid
      id
      url
      videoId
      scriptContent
    }
  `;
}

function optionalArgs(args: string | null) {
  const trimmed = args?.trim();
  return trimmed ? `(${trimmed})` : '';
}

async function runQuery<T>(query: string, variables: Record<string, unknown> = {}): Promise<T> {
  const result = await client.query(query, variables, { requestPolicy: 'network-only' }).toPromise();
  if (result.error) {
    throw new Error(result.error.message);
  }
  return result.data as T;
}

async function runMutation<T>(query: string, variables: Record<string, unknown> = {}): Promise<T> {
  const result = await client.mutation(query, variables).toPromise();
  if (result.error) {
    throw new Error(result.error.message);
  }
  return result.data as T;
}

async function uploadFile(file: File): Promise<string> {
  const createRes = await fetch(`${VARDA_BASE_URL}/files`, {
    method: 'POST',
    headers: {
      'Tus-Resumable': '1.0.0',
      'Upload-Length': String(file.size),
      'Upload-Metadata': `filename ${btoa(file.name)},filetype ${btoa(file.type || 'application/octet-stream')}`,
    },
  });

  if (!createRes.ok) {
    throw new Error(`Upload create failed (${createRes.status})`);
  }

  const uploadPath = createRes.headers.get('Location');
  if (!uploadPath) {
    throw new Error('Missing upload location');
  }

  const patchRes = await fetch(`${VARDA_BASE_URL}${uploadPath}`, {
    method: 'PATCH',
    headers: {
      'Tus-Resumable': '1.0.0',
      'Content-Type': 'application/offset+octet-stream',
      'Upload-Offset': '0',
    },
    body: file,
  });

  if (!patchRes.ok) {
    throw new Error(`Upload failed (${patchRes.status})`);
  }

  const fileUrl = patchRes.headers.get('Varda-File-Url');
  if (!fileUrl) {
    throw new Error('Missing uploaded file URL');
  }

  return fileUrl;
}

export async function uploadImage(file: File): Promise<string> {
  return uploadFile(file);
}

export async function listNotes(
  page: number,
  pageSize: number,
  filters: NoteListFilters,
): Promise<PaginatedNotes> {
  const offset = (page - 1) * pageSize;
  const sortField = filters.sortField ?? 'updatedAt';
  const sortDirection = filters.sortDirection ?? 'DESC';
  const filterBody = buildNoteFilter(filters);

  const query = `
    query ListNotes {
      queryNote${optionalArgs(`
        ${filterBody ? `filter: { ${filterBody} },` : ''}
        sort: { ${sortField}: ${sortDirection} },
        first: ${pageSize},
        offset: ${offset}
      `)} {
        ${buildNoteFields()}
      }
    }
  `;

  const data = await runQuery<{ queryNote: NoteRecord[] }>(query);
  const records = data.queryNote.map((record) => ({
    ...record,
    urls: [],
    images: [],
    tags: [],
    noteVideos: record.noteVideos ?? [],
  }));

  const countQuery = `
    query CountNotes {
      countNote${optionalArgs(filterBody ? `filter: { ${filterBody} }` : null)}
    }
  `;
  const countData = await runQuery<{ countNote: number }>(countQuery);

  return {
    records,
    total: countData.countNote,
  };
}

export async function getNote(noteId: string): Promise<NoteRecord | null> {
  const query = `
    query GetNote {
      getNote(id: ${gqlString(noteId)}) {
        ${buildNoteFields(true)}
      }
    }
  `;

  const data = await runQuery<{ getNote: NoteRecord | null }>(query);
  const record = data.getNote;
  if (!record) {
    return null;
  }
  return {
    ...record,
    urls: [],
    images: [],
    tags: [],
    noteVideos: record.noteVideos ?? [],
  };
}

export async function listRelatedNotes(noteId: string): Promise<RelatedNoteRecord[]> {
  const query = `
    query RelatedNotes {
      queryNoteRelationship {
        relationshipType
        fromNote {
          id
          title
          summary
        }
        toNote {
          id
          title
          summary
        }
      }
    }
  `;

  const data = await runQuery<{
    queryNoteRelationship: Array<{
      relationshipType: string;
      fromNote?: { id?: string | null; title?: string | null; summary?: string | null } | null;
      toNote?: { id?: string | null; title?: string | null; summary?: string | null } | null;
    }>;
  }>(query);

  const related = new Map<string, RelatedNoteRecord>();

  for (const relationship of data.queryNoteRelationship) {
    const fromId = relationship.fromNote?.id ?? null;
    const toId = relationship.toNote?.id ?? null;
    if (fromId !== noteId && toId !== noteId) {
      continue;
    }

    const isOutgoing = fromId === noteId;
    const other = isOutgoing ? relationship.toNote : relationship.fromNote;
    const otherId = other?.id?.trim();
    const otherTitle = other?.title?.trim();

    if (!otherId || !otherTitle) {
      continue;
    }

    const dedupeKey = `${otherId}:${relationship.relationshipType}`;
    if (related.has(dedupeKey)) {
      continue;
    }

    related.set(dedupeKey, {
      id: otherId,
      title: otherTitle,
      summary: other?.summary ?? null,
      relationshipType: relationship.relationshipType,
      direction: isOutgoing ? 'outgoing' : 'incoming',
    });
  }

  return Array.from(related.values()).sort((left, right) => left.title.localeCompare(right.title));
}

export async function listCategories(): Promise<NoteCategory[]> {
  const query = `
    query ListCategories {
      queryNoteCategory(sort: { name: ASC }) {
        ${buildCategoryFields()}
      }
    }
  `;
  const data = await runQuery<{ queryNoteCategory: NoteCategory[] }>(query);
  return data.queryNoteCategory;
}

export async function getCategory(categoryId: string): Promise<NoteCategory | null> {
  const query = `
    query GetCategory {
      getNoteCategory(id: ${gqlString(categoryId)}) {
        ${buildCategoryFields()}
      }
    }
  `;
  const data = await runQuery<{ getNoteCategory: NoteCategory | null }>(query);
  return data.getNoteCategory;
}

export async function createCategory(name: string, parentId?: string | null): Promise<NoteCategory> {
  const mutation = `
    mutation CreateCategory($input: NoteCategoryInput!) {
      createNoteCategory(input: $input) {
        ${buildCategoryFields()}
      }
    }
  `;
  const now = new Date().toISOString();
  const data = await runMutation<{ createNoteCategory: NoteCategory }>(mutation, {
    input: {
      id: crypto.randomUUID(),
      name,
      createdAt: now,
      updatedAt: now,
      parent: parentId ? { id: parentId } : null,
    },
  });
  return data.createNoteCategory;
}

export async function updateCategory(categoryId: string, name: string, parentId?: string | null): Promise<void> {
  const category = await getCategory(categoryId);
  if (!category) {
    throw new Error('Category not found.');
  }

  if (parentId === categoryId) {
    throw new Error('A category cannot be its own parent.');
  }

  const categories = await listCategories();
  const byId = new Map(categories.map((entry) => [entry.id, entry]));
  const seen = new Set<string>();
  let currentId = parentId ?? null;

  while (currentId) {
    if (currentId === categoryId) {
      throw new Error('This parent category would create a cycle.');
    }
    if (seen.has(currentId)) {
      throw new Error('Category hierarchy is invalid.');
    }
    seen.add(currentId);
    currentId = byId.get(currentId)?.parent?.id ?? null;
  }

  const mutation = `
    mutation UpdateCategory {
      updateNoteCategory(
        uid: ${gqlString(category.uid)},
        input: {
          name: ${gqlString(name)},
          updatedAt: ${gqlString(new Date().toISOString())},
          parent: ${gqlString(parentId ? { id: parentId } : null)}
        }
      )
    }
  `;
  await runMutation(mutation);
}

export async function deleteCategory(categoryId: string): Promise<void> {
  const category = await getCategory(categoryId);
  if (!category) {
    throw new Error('Category not found.');
  }
  const mutation = `
    mutation DeleteCategory {
      deleteNoteCategory(uid: ${gqlString(category.uid)})
    }
  `;
  await runMutation(mutation);
}

export async function countNotesForCategory(categoryId: string): Promise<number> {
  const query = `
    query CountNotesForCategory {
      countNote(filter: { category: { id: { eq: ${gqlString(categoryId)} } } })
    }
  `;
  const data = await runQuery<{ countNote: number }>(query);
  return data.countNote;
}

export async function countChildCategories(categoryId: string): Promise<number> {
  const query = `
    query CountChildCategories {
      countNoteCategory(filter: { parent: { id: { eq: ${gqlString(categoryId)} } } })
    }
  `;
  const data = await runQuery<{ countNoteCategory: number }>(query);
  return data.countNoteCategory;
}

export async function listNotesTree(): Promise<NotesTreeData> {
  const query = `
    query NotesTree {
      queryNoteCategory(sort: { name: ASC }) {
        ${buildCategoryFields()}
      }
      queryNote(sort: { title: ASC }) {
        id
        title
        summary
        updatedAt
        category {
          id
        }
      }
    }
  `;

  const data = await runQuery<{
    queryNoteCategory: NoteCategory[];
    queryNote: NotesTreeNote[];
  }>(query);

  return {
    categories: data.queryNoteCategory,
    notes: data.queryNote,
  };
}

export async function checkDuplicateNoteTitle(title: string, excludeId?: string): Promise<boolean> {
  const query = `
    query CheckDuplicate {
      queryNote(filter: { title: { allofterms: ${gqlString(title)} } }) {
        id
      }
    }
  `;

  const data = await runQuery<{ queryNote: Array<{ id: string }> }>(query);
  return data.queryNote.some((record) => record.id !== excludeId);
}

async function deleteNoteVerses(noteId: string): Promise<void> {
  const query = `
    query NoteVerses {
      queryNoteVerse {
        id
        note {
          id
        }
      }
    }
  `;
  const data = await runQuery<{ queryNoteVerse: Array<{ id: string; note?: { id?: string | null } | null }> }>(query);
  for (const verse of data.queryNoteVerse) {
    if (verse.note?.id !== noteId) {
      continue;
    }
    const mutation = `mutation { deleteNoteVerse(uid: ${gqlString(verse.id)}) }`;
    await runMutation(mutation);
  }
}

async function deleteNoteRelationships(noteId: string): Promise<void> {
  const query = `
    query NoteRelationships {
      queryNoteRelationship {
        id
        fromNote {
          id
        }
        toNote {
          id
        }
      }
    }
  `;
  const data = await runQuery<{
    queryNoteRelationship: Array<{
      id: string;
      fromNote?: { id?: string | null } | null;
      toNote?: { id?: string | null } | null;
    }>;
  }>(query);
  for (const relationship of data.queryNoteRelationship) {
    if (relationship.fromNote?.id !== noteId && relationship.toNote?.id !== noteId) {
      continue;
    }
    const mutation = `mutation { deleteNoteRelationship(uid: ${gqlString(relationship.id)}) }`;
    await runMutation(mutation);
  }
}

async function createNoteVerses(noteUid: string, verses: NoteVerseInput[]): Promise<void> {
  for (const verse of verses) {
    const mutation = `
      mutation CreateNoteVerse($input: NoteVerseInput!) {
        createNoteVerse(input: $input) {
          id
        }
      }
    `;

    await runMutation(mutation, {
      input: {
        ...verse,
        note: { uid: noteUid },
      },
    });
  }
}

async function deleteNoteVideos(noteId: string): Promise<void> {
  const query = `
    query NoteVideos {
      queryNoteVideo {
        uid
        note {
          id
        }
      }
    }
  `;
  const data = await runQuery<{ queryNoteVideo: Array<{ uid: string; note?: { id?: string | null } | null }> }>(query);
  for (const video of data.queryNoteVideo) {
    if (video.note?.id !== noteId) {
      continue;
    }
    const mutation = `mutation { deleteNoteVideo(uid: ${gqlString(video.uid)}) }`;
    await runMutation(mutation);
  }
}

async function createNoteVideos(noteUid: string, videos: NoteVideoRecord[]): Promise<void> {
  for (const video of videos) {
    const mutation = `
      mutation CreateNoteVideo($input: NoteVideoInput!) {
        createNoteVideo(input: $input) {
          id
        }
      }
    `;

    await runMutation(mutation, {
      input: {
        id: video.id,
        url: video.url,
        videoId: video.videoId,
        scriptContent: video.scriptContent,
        note: { uid: noteUid },
      },
    });
  }
}

export async function saveNote(input: SaveNoteInput, verses: NoteVerseInput[]): Promise<string> {
  if (input.uid) {
    const mutation = `
      mutation UpdateNote {
        updateNote(
          uid: ${gqlString(input.uid)},
          input: {
            title: ${gqlString(input.title)}
            content: ${gqlString(input.content)}
            urls: ${gqlString(input.urls)}
            images: ${gqlString(input.images)}
            tags: ${gqlString(input.tags)}
            summary: ${gqlString(input.summary)}
            updatedAt: ${gqlString(input.updatedAt)}
            category: { id: ${gqlString(input.categoryId)} }
          }
        )
      }
    `;
    await runMutation(mutation);
    const refreshed = await getNoteByUid(input.uid);
    if (!refreshed) {
      throw new Error('Updated note not found.');
    }
    await deleteNoteVideos(refreshed.id);
    await deleteNoteVerses(refreshed.id);
    await createNoteVerses(refreshed.uid, verses);
    await createNoteVideos(refreshed.uid, input.noteVideos);
    return refreshed.id;
  }

  const mutation = `
    mutation CreateNote($input: NoteInput!) {
      createNote(input: $input) {
        uid
        id
      }
    }
  `;

  const data = await runMutation<{ createNote: { uid: string; id: string } }>(mutation, {
    input: {
      id: crypto.randomUUID(),
      title: input.title,
      content: input.content,
      urls: input.urls,
      images: input.images,
      tags: input.tags,
      summary: input.summary,
      createdAt: input.createdAt ?? input.updatedAt,
      updatedAt: input.updatedAt,
      category: { id: input.categoryId },
    },
  });

  const noteId = data.createNote.id;
  await createNoteVerses(data.createNote.uid, verses);
  await createNoteVideos(data.createNote.uid, input.noteVideos);
  return noteId;
}

export async function deleteNote(noteId: string): Promise<void> {
  const note = await getNote(noteId);
  if (!note) {
    throw new Error('Note not found.');
  }
  await deleteNoteRelationships(note.id);
  await deleteNoteVideos(note.id);
  await deleteNoteVerses(note.id);
  const mutation = `
    mutation DeleteNote {
      deleteNote(uid: ${gqlString(note.uid)})
    }
  `;
  await runMutation(mutation);
}

async function getNoteByUid(uid: string): Promise<NoteRecord | null> {
  const query = `
    query GetNoteByUid {
      getNote(uid: ${gqlString(uid)}) {
        ${buildNoteFields(true)}
      }
    }
  `;
  const data = await runQuery<{ getNote: NoteRecord | null }>(query);
  const record = data.getNote;
  if (!record) {
    return null;
  }
  return {
    ...record,
    urls: [],
    images: [],
    tags: [],
    noteVideos: record.noteVideos ?? [],
  };
}

export async function fetchVerses(
  gospel: string,
  chapter: number,
  start: number,
  end: number,
): Promise<VerseLookupResult[]> {
  const query = `
    query GetVerses($gospel: String!, $chapter: Int!, $start: Int!, $end: Int!) {
      queryUnifiedBible(
        filter: {
          gospel: { eq: $gospel }
          chapter: { eq: $chapter }
          verseNumber: { between: { min: $start, max: $end } }
        }
      ) {
        verseNumber
        text
        chapterHeading
        sectionHeading
      }
    }
  `;

  const result = await client.query(query, { gospel, chapter, start, end }).toPromise();
  if (result.error) {
    throw new Error(result.error.message);
  }

  const verses = (result.data?.queryUnifiedBible ?? []) as VerseLookupResult[];
  return [...verses].sort((a, b) => (a.verseNumber ?? 0) - (b.verseNumber ?? 0));
}
