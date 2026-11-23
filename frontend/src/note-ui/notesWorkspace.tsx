import { useEffect, useMemo, useRef, useState } from 'react';
import { useElementSize } from '@mantine/hooks';
import {
  ActionIcon,
  Alert,
  Badge,
  Box,
  Group,
  Paper,
  Stack,
  Tabs,
  Text,
  ThemeIcon,
} from '@mantine/core';
import { IconAlertCircle, IconChevronDown, IconChevronRight, IconFileText, IconFolders, IconFolderFilled, IconX } from '@tabler/icons-react';
import { Tree, type NodeApi, type NodeRendererProps, type TreeApi } from 'react-arborist';
import { pipe, subscribe } from 'wonka';
import { scriptureClient } from '../lib/client';
import { type NoteCategory, type RelatedNoteRecord, type NotesTreeData, listNotesTree } from './api';
import { NoteViewContent } from './noteViewContent';

type NoteTab = {
  id: string;
  title: string;
};

type TreeNodeData = {
  id: string;
  name: string;
  kind: 'folder' | 'note';
  noteId?: string;
  categoryId?: string | null;
  children?: TreeNodeData[];
};

type WorkspaceTreeData = {
  nodes: TreeNodeData[];
  noteNodeIdByNoteId: Map<string, string>;
  categoryAncestorNodeIds: Map<string, string[]>;
};

type NotesWorkspaceProps = {
  height?: string;
};

type WorkspaceNodeProps = NodeRendererProps<TreeNodeData>;

function noteLabel(title: string) {
  return title.trim() || 'Untitled note';
}

function sortTreeNodes(nodes: TreeNodeData[]) {
  nodes.sort((left, right) => {
    if (left.kind !== right.kind) {
      return left.kind === 'folder' ? -1 : 1;
    }
    return left.name.localeCompare(right.name);
  });
}

function createTreeData(tree: NotesTreeData): WorkspaceTreeData {
  const byId = new Map(tree.categories.map((category) => [category.id, category]));
  const notesByCategoryId = new Map<string | null, NotesTreeData['notes']>();
  const noteNodeIdByNoteId = new Map<string, string>();
  const categoryAncestorNodeIds = new Map<string, string[]>();

  for (const note of tree.notes) {
    const categoryId = note.category?.id ?? null;
    const bucket = notesByCategoryId.get(categoryId) ?? [];
    bucket.push(note);
    notesByCategoryId.set(categoryId, bucket);
  }

  for (const bucket of notesByCategoryId.values()) {
    bucket.sort((left, right) => left.title.localeCompare(right.title));
  }

  const pathCache = new Map<string, string[]>();

  function categorySegments(category: NoteCategory, trail = new Set<string>()): string[] {
    if (pathCache.has(category.id)) {
      return pathCache.get(category.id)!;
    }

    if (trail.has(category.id)) {
      return [category.name];
    }

    trail.add(category.id);
    let segments: string[];

    if (category.parent?.id) {
      const parent = byId.get(category.parent.id);
      const parentSegments = parent ? categorySegments(parent, trail) : [];
      segments = [...parentSegments, category.name];
    } else {
      segments = category.name
        .split('/')
        .map((segment) => segment.trim())
        .filter(Boolean);
    }

    trail.delete(category.id);
    const resolved = segments.length > 0 ? segments : [category.name];
    pathCache.set(category.id, resolved);
    return resolved;
  }

  const rootNodes: TreeNodeData[] = [];

  function appendNoteChildren(parent: TreeNodeData, categoryId: string) {
    const notes = notesByCategoryId.get(categoryId) ?? [];
    const noteChildren = notes.map((note) => {
      const nodeId = `note:${note.id}`;
      noteNodeIdByNoteId.set(note.id, nodeId);
      return {
        id: nodeId,
        name: note.title,
        kind: 'note' as const,
        noteId: note.id,
        categoryId,
      };
    });

    parent.children = [...(parent.children ?? []), ...noteChildren];
    sortTreeNodes(parent.children);
  }

  for (const category of tree.categories) {
    const segments = categorySegments(category);
    let siblings = rootNodes;
    const ancestors: string[] = [];

    for (let index = 0; index < segments.length; index += 1) {
      const segment = segments[index];
      const pathKey = segments.slice(0, index + 1).join('/');
      const nodeId = `folder:${pathKey}`;
      const isLeaf = index === segments.length - 1;

      let node = siblings.find((entry) => entry.id === nodeId);
      if (!node) {
        node = {
          id: nodeId,
          name: segment,
          kind: 'folder',
          categoryId: isLeaf ? category.id : null,
          children: [],
        };
        siblings.push(node);
        sortTreeNodes(siblings);
      } else if (isLeaf) {
        node.categoryId = category.id;
      }

      ancestors.push(nodeId);

      if (isLeaf) {
        appendNoteChildren(node, category.id);
      } else {
        siblings = node.children ?? [];
        node.children = siblings;
      }
    }

    categoryAncestorNodeIds.set(category.id, ancestors);
  }

  const uncategorized = (notesByCategoryId.get(null) ?? []).map((note) => {
    const nodeId = `note:${note.id}`;
    noteNodeIdByNoteId.set(note.id, nodeId);
    return {
      id: nodeId,
      name: note.title,
      kind: 'note' as const,
      noteId: note.id,
      categoryId: null,
    };
  });

  if (uncategorized.length > 0) {
    rootNodes.push({
      id: 'folder:uncategorized',
      name: 'Uncategorized',
      kind: 'folder',
      categoryId: null,
      children: uncategorized,
    });
  }

  sortTreeNodes(rootNodes);

  return { nodes: rootNodes, noteNodeIdByNoteId, categoryAncestorNodeIds };
}

function WorkspaceNode({ node, style, dragHandle }: WorkspaceNodeProps) {
  const isFolder = node.data.kind === 'folder';
  const isSelected = node.isSelected;
  const childNotes = isFolder ? node.data.children?.filter((child) => child.kind === 'note').length ?? 0 : 0;

  return (
    <div style={style}>
      <Group
        ref={dragHandle}
        gap="xs"
        wrap="nowrap"
        px="sm"
        h="100%"
        style={{
          borderRadius: 10,
          marginInline: 6,
          background: isSelected ? '#eef4ff' : 'transparent',
          color: isSelected ? '#1d4ed8' : isFolder ? '#1f2937' : '#4b5563',
          cursor: 'pointer',
        }}
      >
        {isFolder ? (
          <ActionIcon
            variant="subtle"
            color="gray"
            size="sm"
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              node.toggle();
            }}
          >
            {node.isOpen ? <IconChevronDown size={14} /> : <IconChevronRight size={14} />}
          </ActionIcon>
        ) : (
          <Box w={28} />
        )}
        {isFolder ? <IconFolderFilled size={16} stroke={1.8} /> : <IconFileText size={16} stroke={1.8} />}
        <Text size="sm" fw={isFolder ? 600 : 500} lineClamp={1} style={{ flex: 1 }}>
          {node.data.name}
        </Text>
        {isFolder ? (
          <Badge variant="light" color="gray" size="xs">
            {childNotes}
          </Badge>
        ) : null}
      </Group>
    </div>
  );
}

export function NotesWorkspace({ height = 'calc(100vh - 220px)' }: NotesWorkspaceProps) {
  const [tree, setTree] = useState<NotesTreeData>({ categories: [], notes: [] });
  const [error, setError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  const [openTabs, setOpenTabs] = useState<NoteTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const treeApiRef = useRef<TreeApi<TreeNodeData> | null>(null);
  const { ref: treeHostRef, width, height: measuredHeight } = useElementSize();

  const workspaceTree = useMemo(() => createTreeData(tree), [tree]);
  const selectedTreeNodeId = activeTabId ? workspaceTree.noteNodeIdByNoteId.get(activeTabId) ?? undefined : undefined;

  useEffect(() => {
    void (async () => {
      try {
        const nextTree = await listNotesTree();
        setTree(nextTree);
        setError(null);
      } catch (loadError) {
        setError(loadError instanceof Error ? loadError.message : 'Failed to load notes workspace.');
      }
    })();
  }, [refreshTick]);

  useEffect(() => {
    const subscription = pipe(
      scriptureClient.subscription(
        `
          subscription NotesWorkspaceEvents($types: [String]) {
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
    setOpenTabs((current) =>
      current.map((tab) => ({
        ...tab,
        title: tree.notes.find((note) => note.id === tab.id)?.title ?? tab.title,
      })),
    );
  }, [tree.notes]);

  useEffect(() => {
    const treeApi = treeApiRef.current;
    if (!treeApi) {
      return;
    }

    for (const folderId of expandedIds) {
      treeApi.open(folderId);
    }
  }, [expandedIds, workspaceTree.nodes]);

  useEffect(() => {
    if (!activeTabId) {
      return;
    }
    ensureNoteVisible(activeTabId);
  }, [activeTabId]);

  function ensureNoteVisible(noteId: string) {
    const treeApi = treeApiRef.current;
    const treeNodeId = workspaceTree.noteNodeIdByNoteId.get(noteId);
    const categoryId = tree.notes.find((note) => note.id === noteId)?.category?.id ?? null;
    const ancestorIds = categoryId ? workspaceTree.categoryAncestorNodeIds.get(categoryId) ?? [] : ['folder:uncategorized'];

    setExpandedIds((current) => {
      const next = new Set(current);
      for (const ancestorId of ancestorIds) {
        next.add(ancestorId);
      }
      return next;
    });

    if (treeApi && treeNodeId) {
      for (const ancestorId of ancestorIds) {
        treeApi.open(ancestorId);
      }
      treeApi.select(treeNodeId);
    }
  }

  function openNoteTab(noteId: string, title: string) {
    setOpenTabs((current) => (current.some((tab) => tab.id === noteId) ? current : [...current, { id: noteId, title: noteLabel(title) }]));
    setActiveTabId(noteId);
    ensureNoteVisible(noteId);
  }

  async function handleOpenRelatedNote(noteRecord: RelatedNoteRecord) {
    openNoteTab(noteRecord.id, noteRecord.title);
  }

  function closeTab(noteId: string) {
    setOpenTabs((current) => {
      const index = current.findIndex((tab) => tab.id === noteId);
      if (index === -1) {
        return current;
      }

      const next = current.filter((tab) => tab.id !== noteId);
      if (activeTabId === noteId) {
        const fallback = next[index] ?? next[index - 1] ?? null;
        setActiveTabId(fallback?.id ?? null);
      }

      return next;
    });
  }

  function handleActivate(node: NodeApi<TreeNodeData>) {
    if (node.data.kind === 'folder') {
      node.toggle();
      return;
    }

    if (node.data.noteId) {
      openNoteTab(node.data.noteId, node.data.name);
    }
  }

  return (
    <Paper
      radius="xl"
      withBorder
      style={{
        height,
        overflow: 'hidden',
        background: '#f8f5ef',
      }}
    >
      <Group h="100%" wrap="nowrap" align="stretch" gap={0}>
        <Box
          style={{
            width: 340,
            minWidth: 300,
            borderRight: '1px solid #e5ded4',
            display: 'flex',
            flexDirection: 'column',
            background: '#ffffff',
          }}
        >
          <Group px="md" py="sm" gap="sm" style={{ borderBottom: '1px solid #ece6dc' }}>
            <ThemeIcon variant="light" color="gray" radius="xl">
              <IconFolders size={16} />
            </ThemeIcon>
            <Box style={{ flex: 1 }}>
              <Text size="sm" fw={700} c="#111827">
                Notes Tree
              </Text>
              <Text size="xs" c="#6b7280">
                Categories and notes
              </Text>
            </Box>
          </Group>

          {error ? (
            <Alert color="red" icon={<IconAlertCircle size={16} />} m="md">
              {error}
            </Alert>
          ) : null}

          <Box ref={treeHostRef} style={{ flex: 1, minHeight: 0 }}>
            {width > 0 && measuredHeight > 0 ? (
              <Tree<TreeNodeData>
                ref={treeApiRef}
                data={workspaceTree.nodes}
                width={width}
                height={measuredHeight}
                rowHeight={38}
                indent={20}
                padding={8}
                openByDefault={false}
                disableDrag
                disableEdit
                selection={selectedTreeNodeId}
                onActivate={handleActivate}
                onToggle={(id) => {
                  setExpandedIds((current) => {
                    const next = new Set(current);
                    if (next.has(id)) {
                      next.delete(id);
                    } else {
                      next.add(id);
                    }
                    return next;
                  });
                }}
              >
                {WorkspaceNode}
              </Tree>
            ) : null}
          </Box>
        </Box>

        <Box style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', background: '#f8f5ef' }}>
          {openTabs.length === 0 ? (
            <Stack align="center" justify="center" gap="sm" style={{ flex: 1 }}>
              <ThemeIcon variant="light" color="blue" size={48} radius="xl">
                <IconFileText size={24} />
              </ThemeIcon>
              <Text fw={700}>Open a note from the tree</Text>
              <Text c="dimmed" size="sm">
                The selected note will appear here in a reusable tab.
              </Text>
            </Stack>
          ) : (
            <Tabs
              value={activeTabId}
              onChange={setActiveTabId}
              style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}
            >
              <Box style={{ borderBottom: '1px solid #d8d5cc', background: '#ebe6dd' }}>
                <Tabs.List>
                  {openTabs.map((tab) => (
                    <Tabs.Tab
                      key={tab.id}
                      value={tab.id}
                      rightSection={
                        <ActionIcon
                          variant="subtle"
                          color="gray"
                          size="sm"
                          aria-label={`Close ${tab.title}`}
                          onClick={(event) => {
                            event.preventDefault();
                            event.stopPropagation();
                            closeTab(tab.id);
                          }}
                        >
                          <IconX size={14} />
                        </ActionIcon>
                      }
                    >
                      <Text size="sm" lineClamp={1}>
                        {tab.title}
                      </Text>
                    </Tabs.Tab>
                  ))}
                </Tabs.List>
              </Box>

              <Box style={{ flex: 1, minHeight: 0, overflow: 'auto', padding: 16 }}>
                {openTabs.map((tab) => (
                  <Tabs.Panel key={tab.id} value={tab.id} style={{ height: '100%' }}>
                    <NoteViewContent noteId={tab.id} embedded onOpenNote={handleOpenRelatedNote} />
                  </Tabs.Panel>
                ))}
              </Box>
            </Tabs>
          )}
        </Box>
      </Group>
    </Paper>
  );
}
