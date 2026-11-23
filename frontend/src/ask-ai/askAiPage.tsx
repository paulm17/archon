
import React, { useCallback, useRef, useState, useEffect } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  Background,
  ConnectionLineType,
  Handle,
  MarkerType,
  Panel,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesState,
  useReactFlow,
  useUpdateNodeInternals,
  type Connection,
  type Edge,
  type Node,
  type NodeProps,
  type NodeTypes,
  type OnConnectEnd,
  type OnConnectStartParams,
  type XYPosition,
} from '@xyflow/react';

import '@xyflow/react/dist/style.css';

import {
  ActionIcon,
  Box,
  Button,
  Group,
  Loader,
  Paper,
  Popover,
  Stack,
  Text,
  Textarea,
  Collapse,
} from '@mantine/core';

import {
  IconBolt,
  IconDots,
  IconMaximize,
  IconMenu2,
  IconNetwork,
  IconPlus,
  IconZoomIn,
  IconZoomOut,
} from '@tabler/icons-react';
import ELK from 'elkjs/lib/elk.bundled.js';

const elk = new ELK();

interface Source {
  id: string;
  title: string;
  snippet: string;
  score: number;
}

interface QuestionNodeData extends Record<string, unknown> {
  question: string;
  answer?: string;
  thinking?: string;
  sources?: Source[];
  hasAnswer: boolean;
  hasChildren: boolean;
  isOptions: boolean;
  isInvalidated: boolean;
  isRoot?: boolean;
  getContext?: (nodeId: string) => Array<{ id: string; question: string; answer?: string }>;
  onAnswered?: (nodeId: string, question: string, answer: string) => void;
}

type QuestionNodeType = Node<QuestionNodeData>;

type FlowEdge = Edge<QuestionNodeData>;

const getLayoutedElements = async (
  nodes: QuestionNodeType[],
  edges: FlowEdge[]
): Promise<{ nodes: QuestionNodeType[]; edges: FlowEdge[] }> => {
  const graph = {
    id: 'root',
    layoutOptions: {
      'elk.algorithm': 'layered',
      'elk.direction': 'DOWN',
      'elk.spacing.nodeNode': '80',
      'elk.layered.spacing.nodeNodeBetweenLayers': '100',
    },
    children: nodes.map((node) => ({
      id: node.id,
      width: 350,
      height: 200, // Approximate height, ELK might need more accurate sizing
    })),
    edges: edges.map((edge) => ({
      id: edge.id,
      sources: [edge.source],
      targets: [edge.target],
    })),
  };

  const layoutedGraph = await elk.layout(graph);

  const layoutedNodes = nodes.map((node) => {
    const layoutedNode = layoutedGraph.children?.find((n) => n.id === node.id);
    if (!layoutedNode) {
      return node;
    }
    return {
      ...node,
      position: {
        x: layoutedNode.x ?? node.position.x,
        y: layoutedNode.y ?? node.position.y,
      },
    } as QuestionNodeType;
  });

  return { nodes: layoutedNodes, edges };
};

// Helper function to convert AI output to proper markdown
const convertToMarkdown = (text: string) => {
  let markdown = text;
  markdown = markdown.replace(/([.!?:])\s+(\d+)\.\s+/g, '$1\n\n$2. ');
  markdown = markdown.replace(/(\w)\s+(\d+)\.\s+\*\*/g, '$1\n\n$2. **');
  return markdown;
};

const QuestionNode = ({ data, id }: NodeProps<QuestionNodeType>) => {
  const [isLoading, setIsLoading] = useState(false);
  const [hasAnswer, setHasAnswer] = useState(data.hasAnswer || false);
  const [question, setQuestion] = useState(data.question || '');
  const [answer, setAnswer] = useState(data.answer || '');
  const [thinking, setThinking] = useState(data.thinking || '');
  const [_sources, setSources] = useState<Source[]>(data.sources || []);
  const [showOptions, setShowOptions] = useState(false);
  const [isInvalidated, setIsInvalidated] = useState(data.isInvalidated || false);
  const [thinkingCollapsed, setThinkingCollapsed] = useState(true);
  const [thinkingLoading, setThinkingLoading] = useState(false);

  const updateNodeInternals = useUpdateNodeInternals();
  const hasChildren = data.hasChildren || false;
  const isOptionsNode = data.isOptions || false;
  const answerRef = useRef<HTMLDivElement>(null);
  const answerAccRef = useRef(''); // Accumulated answer text (avoids stale closure)
  const thinkingRef = useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    setIsInvalidated(data.isInvalidated || false);
  }, [data.isInvalidated]);

  React.useEffect(() => {
    setHasAnswer(data.hasAnswer || false);
    updateNodeInternals(id);
  }, [data.hasAnswer, id, updateNodeInternals]);

  // Also update internals if local hasAnswer changes
  React.useEffect(() => {
    updateNodeInternals(id);
  }, [hasAnswer, id, updateNodeInternals]);

  // Auto-scroll thinking box
  useEffect(() => {
    if (thinkingRef.current && thinkingLoading && !thinkingCollapsed) {
      thinkingRef.current.scrollTop = thinkingRef.current.scrollHeight;
    }
  }, [thinking, thinkingLoading, thinkingCollapsed]);

  // Auto-scroll answer box
  useEffect(() => {
    if (answerRef.current && isLoading) {
      answerRef.current.scrollTop = answerRef.current.scrollHeight;
    }
  }, [answer, isLoading]);

  const handleSubmit = useCallback(async () => {
    if (!question.trim()) return;

    setIsLoading(true);
    setAnswer('');
    answerAccRef.current = '';
    setThinking('');
    setSources([]);
    setThinkingLoading(false);

    // Build context from ancestors
    const ancestors = data.getContext?.(id) || [];
    const messages = [];

    // Add ancestor context as history
    for (const p of ancestors) {
      messages.push({ role: 'user', content: p.question });
      if (p.answer) {
        messages.push({ role: 'assistant', content: p.answer });
      }
    }

    // Add current question
    messages.push({ role: 'user', content: question });

    console.log('Submitting question (Mock):', {
      nodeId: id,
      question,
      messages
    });

    try {
      setThinkingLoading(true);

      // Real implementation connecting to Archon Brain
      // Port 3001 is hardcoded in main.rs for now.
      // In production, this should be configurable or proxied.
      const response = await fetch(`${import.meta.env.VITE_API_BASE_URL || 'http://localhost:3001'}/ask-ai`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          query: question,
          conversation_id: "test-conv-id",
          history: messages.slice(0, -1), // All ancestor Q&A pairs (exclude current question)
        }),
      });

      if (!response.ok) throw new Error(`API error: ${response.status}`);
      if (!response.body) throw new Error('No response body');

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      while (true) {
        const { value, done } = await reader.read();
        if (done) break;

        const chunk = decoder.decode(value, { stream: true });
        buffer += chunk;

        // Parse SSE events
        // format: data: {...}\n\n
        const lines = buffer.split('\n\n');
        buffer = lines.pop() || ''; // Keep the last incomplete chunk

        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed.startsWith('data: ')) continue;

          const jsonStr = trimmed.slice(6);
          if (!jsonStr) continue;

          try {
            const event = JSON.parse(jsonStr);

            switch (event.type) {
              case 'thought':
                // event.payload is Thought { title, content, state }
                // We append content for now or just show the latest
                setThinking(prev => prev + (event.payload.content || '') + '\n');
                break;
              case 'token':
                answerAccRef.current += event.payload;
                setAnswer(prev => prev + event.payload);
                // Hide thinking when answer starts?
                if (thinkingLoading) setThinkingLoading(false);
                break;
              case 'source':
                setSources(prev => [...prev, event.payload]);
                break;
              case 'error':
                setAnswer(prev => prev + '\n\n**Error:** ' + event.payload);
                setIsLoading(false);
                break;
              case 'done':
                setIsLoading(false);
                setHasAnswer(true);
                setIsInvalidated(false);
                data.onAnswered?.(id, question, answerAccRef.current);
                break;
            }
          } catch (e) {
            console.error("Failed to parse SSE line", e);
          }
        }
      }

    } catch (error) {
      console.error('Error submitting question:', error);
      setAnswer('Error: Failed to get response. Is the Brain service running?');
    } finally {
      setIsLoading(false);
      setThinkingLoading(false);
    }

  }, [question, id, data]);

  const handleOptionClick = useCallback(
    (option: string) => {
      console.log(`Option clicked: ${option} for node ${id}`);
      setShowOptions(false);
    },
    [id]
  );

  return (
    <Paper
      shadow="md"
      p="md"
      radius="xl"
      style={{ minWidth: 350, maxWidth: 450, border: '2px solid #dee2e6', position: 'relative' }}
    >
      <Stack gap="md">
        <Box>
          <Group gap="xs" align="flex-start" wrap="nowrap">
            {isOptionsNode && (
              <ActionIcon
                size="sm"
                variant="subtle"
                onClick={() => setShowOptions(!showOptions)}
                style={{ marginTop: 4, flexShrink: 0 }}
              >
                <IconDots size={16} />
              </ActionIcon>
            )}
            <Textarea
              value={question}
              onChange={(e) => setQuestion(e.target.value)}
              disabled={hasAnswer || isLoading}
              placeholder="What can I help with?"
              minRows={2}
              autosize
              style={{ flex: 1 }}
            />
            <ActionIcon
              onClick={handleSubmit}
              disabled={(hasAnswer && !isInvalidated) || hasChildren || !question.trim() || isLoading}
              size="lg"
              variant={hasChildren ? 'filled' : isInvalidated ? 'filled' : 'light'}
              color={hasChildren ? 'red' : isInvalidated ? 'orange' : 'gray'}
              style={{ marginTop: 0 }}
            >
              {isLoading ? <Loader size="xs" /> : <IconBolt size={20} />}
            </ActionIcon>
          </Group>

          {showOptions && isOptionsNode && (
            <Group gap="xs" wrap="wrap" mt="xs">
              <Button variant="light" size="xs" onClick={() => handleOptionClick('More concise')}>
                More concise
              </Button>
              <Button variant="light" size="xs" onClick={() => handleOptionClick('Add details')}>
                Add details
              </Button>
              <Button variant="light" size="xs" onClick={() => handleOptionClick('ELI5')}>
                ELI5
              </Button>
              <Button variant="light" size="xs" onClick={() => handleOptionClick('Emojify')}>
                Emojify
              </Button>
            </Group>
          )}
        </Box>

        {(thinking || thinkingLoading) && (
          <Box>
            <Group justify="space-between" mb={4} onClick={() => setThinkingCollapsed(!thinkingCollapsed)} style={{ cursor: 'pointer' }}>
              <Text size="xs" c="dimmed" fw={700}>Thinking Process</Text>
              <Text size="xs" c="dimmed">{thinkingCollapsed ? 'Show' : 'Hide'}</Text>
            </Group>
            <Collapse in={!thinkingCollapsed}>
              <Paper p="xs" bg="gray.0" style={{ maxHeight: 100, overflowY: 'auto' }} ref={thinkingRef}>
                <Text size="xs" c="dimmed" style={{ whiteSpace: 'pre-wrap' }}>{thinking}</Text>
                {thinkingLoading && <Loader size="xs" mt={4} />}
              </Paper>
            </Collapse>
          </Box>
        )}

        {(hasAnswer || answer) && (
          <Paper p="sm" bg="blue.0" radius="sm" style={{ opacity: isInvalidated ? 0.4 : 1 }}>
            <Box
              ref={answerRef}
              style={{
                lineHeight: '24px',
                fontSize: '14px',
                maxHeight: '300px',
                overflowY: 'auto'
              }}
            >
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                components={{
                  p: ({ children }) => <div style={{ marginBottom: '8px' }}>{children}</div>,
                  ol: ({ children }) => (
                    <ol style={{ marginTop: '8px', marginBottom: '8px' }}>{children}</ol>
                  ),
                  ul: ({ children }) => (
                    <ul style={{ marginTop: '8px', marginBottom: '8px' }}>{children}</ul>
                  ),
                }}
              >
                {convertToMarkdown(answer)}
              </ReactMarkdown>
            </Box>
          </Paper>
        )}
      </Stack>

      {!data.isRoot && (
        <Handle
          type="target"
          position={Position.Top}
          style={{
            width: 20,
            height: 20,
            background: '#228be6',
            border: '2px solid white',
            top: -6,
          }}
        />
      )}

      {hasAnswer && (
        <Handle
          type="source"
          position={Position.Bottom}
          style={{
            width: 20,
            height: 20,
            background: '#228be6',
            border: '2px solid white',
            bottom: -6,
          }}
        />
      )}
    </Paper>
  );
};

const nodeTypes: NodeTypes = {
  question: QuestionNode,
};

const FlowEditor = () => {
  const [nodes, setNodes, onNodesChange] = useNodesState<QuestionNodeType>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<FlowEdge>([]);
  const { zoomIn, zoomOut, fitView, screenToFlowPosition } = useReactFlow();
  const [instructionText, setInstructionText] = useState('');
  const [showInstructionInput, setShowInstructionInput] = useState(false);
  const connectingNodeId = useRef<string | null>(null);
  const justConnected = useRef(false);

  const nodesRef = useRef(nodes);
  const edgesRef = useRef(edges);

  useEffect(() => {
    nodesRef.current = nodes;
  }, [nodes]);

  useEffect(() => {
    edgesRef.current = edges;
  }, [edges]);

  const getContextNodes = useCallback(
    (nodeId: string) => {
      console.log('getContextNodes called for:', nodeId);
      const ancestors: Array<{ id: string; question: string; answer?: string }> = [];
      const visited = new Set<string>();
      const currentNodes = nodesRef.current;
      const currentEdges = edgesRef.current;

      const traverse = (currentId: string) => {
        const parentEdges = currentEdges.filter((e) => e.target === currentId);

        const parentNodes = parentEdges
          .map((e) => currentNodes.find((n) => n.id === e.source))
          .filter((n): n is QuestionNodeType => Boolean(n));

        for (const parent of parentNodes) {
          if (!visited.has(parent.id)) {
            visited.add(parent.id);
            // Recurse first to get older ancestors first (Root -> Child)
            traverse(parent.id);
            ancestors.push({
              id: parent.id,
              question: parent.data.question,
              answer: parent.data.answer
            });
          }
        }
      };

      traverse(nodeId);
      return ancestors;
    },
    []
  );

  const updateNodeChildren = useCallback(() => {
    setNodes((nds) =>
      nds.map((node) => {
        const hasChildren = edges.some((e) => e.source === node.id);
        const updatedNode: QuestionNodeType = {
          ...node,
          data: {
            ...node.data,
            hasChildren,
          },
        };
        return updatedNode;
      })
    );
  }, [edges, setNodes]);

  const invalidateDescendants = useCallback(
    (nodeId: string) => {
      const visited = new Set<string>();
      const toInvalidate = new Set<string>();

      const findDescendants = (currentId: string) => {
        if (visited.has(currentId)) return;
        visited.add(currentId);

        const childEdges = edges.filter((e) => e.source === currentId);
        childEdges.forEach((edge) => {
          toInvalidate.add(edge.target);
          findDescendants(edge.target);
        });
      };

      findDescendants(nodeId);

      setNodes((nds) =>
        nds.map((node) => {
          if (toInvalidate.has(node.id)) {
            return {
              ...node,
              data: {
                ...node.data,
                isInvalidated: true,
              },
            } as QuestionNodeType;
          }
          return node;
        })
      );
    },
    [edges, setNodes]
  );

  const createNode = useCallback(
    (position: XYPosition, isOptions = false, isRoot = false) => {
      const id = `node_${crypto.randomUUID()}`;
      const newNode: QuestionNodeType = {
        id,
        type: 'question',
        position,
        data: {
          question: '',
          isOptions,
          isRoot,
          hasChildren: false,
          hasAnswer: false,
          isInvalidated: false,
          getContext: getContextNodes,
          onAnswered: (nodeId: string, questionText: string, answerText: string) => {
            setNodes((nds) =>
              nds.map((n) => {
                if (n.id === nodeId) {
                  return {
                    ...n,
                    data: {
                      ...n.data,
                      question: questionText,
                      answer: answerText,
                      hasAnswer: true,
                      isInvalidated: false,
                    },
                  } as QuestionNodeType;
                }
                return n;
              })
            );
            invalidateDescendants(nodeId);
            updateNodeChildren();
          },
        },
      };

      setNodes((nds) => [...nds, newNode]);
      return id;
    },
    [setNodes, getContextNodes, updateNodeChildren, invalidateDescendants]
  );

  const handleNewInstruction = useCallback(() => {
    const position: XYPosition =
      nodes.length === 0
        ? { x: window.innerWidth / 2 - 175, y: 100 }
        : { x: 250, y: nodes.length * 250 + 100 };

    createNode(position, false, true);
  }, [nodes, createNode]);

  const handleSaveInstruction = useCallback(() => {
    console.log('General instruction saved:', instructionText);
    setShowInstructionInput(false);
    setInstructionText('');
  }, [instructionText]);

  const onConnect = useCallback(
    (params: Connection) => {
      if (!params.source || !params.target) return;

      justConnected.current = true;
      // Reset flag after a short delay
      setTimeout(() => {
        justConnected.current = false;
      }, 100);

      const newEdge: FlowEdge = {
        id: `edge_${crypto.randomUUID()}`,
        source: params.source,
        target: params.target,
        sourceHandle: params.sourceHandle,
        targetHandle: params.targetHandle,
        type: 'smoothstep',
        animated: true,
        markerEnd: {
          type: MarkerType.ArrowClosed,
        },
      };
      setEdges((eds) => [...eds, newEdge]);
      updateNodeChildren();
    },
    [setEdges, updateNodeChildren]
  );

  const onConnectStart = useCallback((_: MouseEvent | TouchEvent, { nodeId }: OnConnectStartParams) => {
    connectingNodeId.current = nodeId ?? null;
  }, []);

  const onConnectEnd = useCallback<OnConnectEnd>(
    (event) => {
      if (!connectingNodeId.current) return;

      if (justConnected.current) {
        connectingNodeId.current = null;
        return;
      }

      const target = event.target as Element | null;
      const targetIsPane = target?.classList?.contains('react-flow__pane');
      const targetIsHandle = target?.classList?.contains('react-flow__handle') || target?.closest('.react-flow__handle');

      if (targetIsPane && !targetIsHandle) {
        let clientX: number | null = null;
        let clientY: number | null = null;

        if ('clientX' in event) {
          clientX = event.clientX;
          clientY = event.clientY;
        } else if ('changedTouches' in event && event.changedTouches.length > 0) {
          clientX = event.changedTouches[0].clientX;
          clientY = event.changedTouches[0].clientY;
        }

        if (clientX == null || clientY == null) {
          connectingNodeId.current = null;
          return;
        }

        const position = screenToFlowPosition({
          x: clientX,
          y: clientY,
        });

        const newNodeId = createNode(position, true, false);

        const newEdge: FlowEdge = {
          id: `edge_${crypto.randomUUID()}`,
          source: connectingNodeId.current,
          target: newNodeId,
          type: 'smoothstep',
          animated: true,
          markerEnd: {
            type: MarkerType.ArrowClosed,
          },
        };

        setEdges((eds) => [...eds, newEdge]);
        updateNodeChildren();
      }

      connectingNodeId.current = null;
    },
    [screenToFlowPosition, createNode, setEdges, updateNodeChildren]
  );

  const handleOrganize = useCallback(async () => {
    const { nodes: layoutedNodes, edges: layoutedEdges } = await getLayoutedElements(nodes, edges);

    setNodes(layoutedNodes);
    setEdges(layoutedEdges);

    window.requestAnimationFrame(() => {
      fitView({ padding: 0.2, duration: 400 });
    });
  }, [nodes, edges, setNodes, setEdges, fitView]);

  const handleQuickAction = useCallback(
    (action: string) => {
      if (action === 'Resolve all') {
        const invalidatedNodes = nodes.filter((n) => n.data.isInvalidated);
        invalidatedNodes.forEach((node) => {
          setTimeout(() => {
            setNodes((nds) =>
              nds.map((n) => {
                if (n.id === node.id) {
                  return {
                    ...n,
                    data: {
                      ...n.data,
                      isInvalidated: false,
                    },
                  } as QuestionNodeType;
                }
                return n;
              })
            );
          }, 1000);
        });
      }
    },
    [nodes, setNodes] // Removed getContextNodes dependency as it wasn't used in this simplified logic
  );

  return (
    <Box
      style={{
        width: '100%',
        height: 'calc(100vh - 120px)', // Adjusted for Layout
        position: 'relative',
        overflow: 'hidden',
      }}
    >
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onConnectStart={onConnectStart}
        onConnectEnd={onConnectEnd}
        nodeTypes={nodeTypes}
        defaultViewport={{ x: 0, y: 0, zoom: 1.0 }}
        connectionLineStyle={{ stroke: '#228be6', strokeWidth: 2 }}
        connectionLineType={ConnectionLineType.SmoothStep}
      >
        <Background />

        <Panel position="top-left">
          <Group gap="xs" style={{ position: 'relative' }}>
            <Button leftSection={<IconPlus size={16} />} onClick={handleNewInstruction} size="sm">
              New instruction
            </Button>

            {showInstructionInput && (
              <Paper
                p="md"
                shadow="sm"
                style={{ width: 300, position: 'absolute', top: 40, left: 0, zIndex: 1000 }}
              >
                <Stack gap="sm">
                  <Textarea
                    value={instructionText}
                    onChange={(e) => setInstructionText(e.target.value)}
                    placeholder="Enter general instruction..."
                    minRows={4}
                    autosize
                  />
                  <Group gap="xs" justify="flex-end">
                    <Button size="xs" onClick={handleSaveInstruction}>
                      Save
                    </Button>
                    <Button
                      size="xs"
                      variant="outline"
                      onClick={() => setShowInstructionInput(false)}
                    >
                      Cancel
                    </Button>
                  </Group>
                </Stack>
              </Paper>
            )}

            <Button variant="outline" size="sm" onClick={() => setShowInstructionInput(true)}>
              General instructions
            </Button>

            <Popover position="right" withArrow shadow="md">
              <Popover.Target>
                <Button variant="outline" size="sm" leftSection={<IconMenu2 size={16} />}>
                  Quick actions
                </Button>
              </Popover.Target>
              <Popover.Dropdown>
                <Stack gap="xs">
                  <Button
                    variant="light"
                    size="xs"
                    fullWidth
                    onClick={() => handleQuickAction('Resolve all')}
                    leftSection={<IconBolt size={14} />}
                  >
                    Resolve all
                  </Button>
                </Stack>
              </Popover.Dropdown>
            </Popover>
          </Group>
        </Panel>

        <Panel position="bottom-left">
          <Stack gap="xs">
            <ActionIcon size="lg" variant="filled" onClick={() => zoomIn({ duration: 300 })}>
              <IconZoomIn size={20} />
            </ActionIcon>
            <ActionIcon size="lg" variant="filled" onClick={() => zoomOut({ duration: 300 })}>
              <IconZoomOut size={20} />
            </ActionIcon>
            <ActionIcon
              size="lg"
              variant="filled"
              onClick={() => fitView({ padding: 0.2, duration: 400 })}
            >
              <IconMaximize size={20} />
            </ActionIcon>
            <ActionIcon size="lg" variant="filled" onClick={handleOrganize}>
              <IconNetwork size={20} />
            </ActionIcon>
          </Stack>
        </Panel>
      </ReactFlow>
    </Box>
  );
};

export const AskAiPage = () => {
  return (
    <ReactFlowProvider>
      <FlowEditor />
    </ReactFlowProvider>
  );
};
