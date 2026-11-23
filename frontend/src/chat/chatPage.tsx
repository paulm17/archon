import React, { useState, useRef, useEffect, useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  Box,
  Container,
  Group,
  Stack,
  Text,
  TextInput,
  Textarea,
  ActionIcon,
  ScrollArea,
  Paper,
  Avatar,
  Button,
  UnstyledButton,
  Loader,
  rem,
} from '@mantine/core';
import {
  IconPlus,
  IconSearch,
  IconPaperclip,
  IconMessage,
  IconEdit,
  IconSettings,
  IconArrowUp,
  IconMicrophone,
  IconSquare,
} from '@tabler/icons-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { executeVardaRequest } from '../lib/client';
import { useChatStore } from '../lib/store';

interface Message {
  role: 'user' | 'assistant' | 'system';
  content: string;
}

interface ChatThread {
  id: string;
  title?: string;
  createdAt: string;
}

export function ChatPage() {
  const navigate = useNavigate();
  const { currentThreadId: storedThreadId, setCurrentThreadId } = useChatStore();

  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [modelName, setModelName] = useState<string | null>(null);
  const [threadId, setThreadId] = useState<string | null>(storedThreadId);
  const [threads, setThreads] = useState<ChatThread[]>([]);
  const [loadingTitleIds, setLoadingTitleIds] = useState<Set<string>>(new Set());
  const viewport = useRef<HTMLDivElement>(null);
  const abortControllerRef = useRef<AbortController | null>(null);

  // Sync threadId changes back to the store
  useEffect(() => {
    setCurrentThreadId(threadId);
  }, [threadId, setCurrentThreadId]);

  // Sidebar state
  const [searchQuery, setSearchQuery] = useState('');

  // Scroll to bottom when messages change
  const scrollToBottom = useCallback(() => {
    if (viewport.current) {
      viewport.current.scrollTo({ top: viewport.current.scrollHeight, behavior: 'smooth' });
    }
  }, []);

  useEffect(() => {
    scrollToBottom();
  }, [messages, scrollToBottom]);

  // Check for loaded model on mount
  useEffect(() => {
    const checkModel = async () => {
      try {
        const res = await fetch('http://localhost:8080/v1/models');
        if (res.ok) {
          const data = await res.json();
          if (data.data && data.data.length > 0) {
            setModelName(data.data[0].id);
          }
        }
      } catch (e) {
        console.error('Failed to check model status', e);
      }
    };
    checkModel();
    fetchThreads().then(() => {
      // Restore previous thread on mount
      if (storedThreadId) {
        loadThread(storedThreadId);
      }
    });
  }, []);

  const fetchThreads = async () => {
    try {
      const data = await executeVardaRequest<{ queryChatThread: ChatThread[] }>(
        'chat',
        `
        query GetThreads {
          queryChatThread(first: 50) {
            id
            title
            createdAt
          }
        }
        `
      );
      setThreads(data.queryChatThread.slice().sort((a, b) => b.createdAt.localeCompare(a.createdAt)));
    } catch (e) {
      console.error('Failed to fetch threads', e);
    }
  };

  const loadThread = async (tid: string) => {
    setThreadId(tid);
    setMessages([]);
    setIsLoading(true);
    try {
      const data = await executeVardaRequest<{
        getChatThread: { messages: { role: string; content: string; createdAt: string }[] | null }
      }>(
        'chat',
        `
  query GetThreadMessages($tid: ID!) {
    getChatThread(id: $tid) {
      messages {
        role
        content
        createdAt
      }
    }
  }
  `,
        { tid }
      );

      const msgs = (data.getChatThread?.messages ?? [])
        .sort((a, b) => a.createdAt.localeCompare(b.createdAt))
        .map(m => ({ role: m.role as Message['role'], content: m.content }));

      setMessages(msgs);
    } catch (e) {
      console.error('Failed to load thread messages', e);
    } finally {
      setIsLoading(false);
    }
  };

  const handleNewChat = () => {
    setMessages([]);
    setInput('');
    setThreadId(null);
    setIsLoading(false);
  };

  const generateId = () => {
    try {
      if (typeof crypto?.randomUUID === 'function') return crypto.randomUUID();
    } catch (e) { /* fallback */ }
    return Date.now().toString(36) + Math.random().toString(36).substring(2);
  };

  const updateThreadTitle = async (tuid: string, title: string) => {
    try {
      await executeVardaRequest(
        'chat',
        `
        mutation UpdateThread($uid: ID!, $input: ChatThreadInput!) {
          updateChatThread(uid: $uid, input: $input)
        }
        `,
        { uid: tuid, input: { title } }
      );
      setThreads(prev => prev.map(t => t.id === tuid ? { ...t, title } : t));
      setLoadingTitleIds(prev => {
        const next = new Set(prev);
        next.delete(tuid);
        return next;
      });
    } catch (e) {
      console.error('Failed to update thread title', e);
      // Clear loading state even on error to avoid stuck spinner
      setLoadingTitleIds(prev => {
        const next = new Set(prev);
        next.delete(tuid);
        return next;
      });
    }
  };

  const saveMessage = async (tuid: string, role: string, content: string) => {
    const now = new Date().toISOString();
    try {
      // Add the message
      await executeVardaRequest(
        'chat',
        `
        mutation AddMessage($input: ChatMessageInput!) {
          createChatMessage(input: $input) {
            id
          }
        }
        `,
        {
          input: {
            id: generateId(),
            thread: { id: tuid },
            role,
            content,
            createdAt: now,
          }
        }
      );
    } catch (e) {
      console.error('Failed to save message', e);
    }
  };

  const createThread = async (): Promise<string> => {
    const now = new Date().toISOString();
    try {
      const data = await executeVardaRequest<{ createChatThread: { id: string } }>(
        'chat',
        `
        mutation CreateThread($input: ChatThreadInput!) {
          createChatThread(input: $input) {
            id
          }
        }
        `,
        { input: { createdAt: now } }  // Don't pass id — let the server assign it
      );
      return data.createChatThread.id;
    } catch (e) {
      console.error('Failed to create thread', e);
      throw e;
    }
  };

  const handleSubmit = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    if (!input.trim() || isLoading) return;

    const userMessage: Message = { role: 'user', content: input };
    const newMessages = [...messages, userMessage];
    setMessages(newMessages);
    setInput('');
    setIsLoading(true);

    let currentThreadId = threadId;
    if (!currentThreadId) {
      currentThreadId = await createThread();
      setThreadId(currentThreadId);
      const newThread: ChatThread = { id: currentThreadId!, createdAt: new Date().toISOString() };
      setThreads(prev => [newThread, ...prev]);
      setLoadingTitleIds(prev => new Set(prev).add(currentThreadId!));
    }

    // Persist user message
    await saveMessage(currentThreadId, 'user', input);

    let assistantContent = '';
    try {
      abortControllerRef.current = new AbortController();
      const response = await fetch('http://localhost:8080/v1/chat/completions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          messages: newMessages,
          stream: true,
          generate_title: messages.length === 0,
        }),
        signal: abortControllerRef.current.signal,
      });

      if (!response.ok) throw new Error(`API error: ${response.status}`);
      if (!response.body) throw new Error('No response body');

      const reader = response.body.getReader();
      const decoder = new TextDecoder();

      setMessages(prev => [...prev, { role: 'assistant', content: '' }]);

      while (true) {
        const { value, done } = await reader.read();
        if (done) break;

        const chunk = decoder.decode(value, { stream: true });
        const lines = chunk.split('\n');

        for (const line of lines) {
          const trimmed = line.trim();
          if (trimmed === 'data: [DONE]') break;
          const dataPrefix = 'data: ';
          if (trimmed.startsWith(dataPrefix)) {
            try {
              const data = JSON.parse(trimmed.slice(dataPrefix.length));

              // Handle generated title if present
              if (data.title && currentThreadId) {
                console.log('Raw title:', data.title);
                updateThreadTitle(currentThreadId, data.title);
              }

              const delta = data.choices[0].delta?.content ?? '';
              assistantContent += delta;

              setMessages(prev => {
                const updated = [...prev];
                updated[updated.length - 1] = { role: 'assistant', content: assistantContent };
                return updated;
              });
            } catch (e) {
              // Ignore parse errors for partial chunks
            }
          }
        }
      }
    } catch (error) {
      if (error instanceof DOMException && error.name === 'AbortError') return;
      console.error('Chat error:', error);
      setMessages(prev => [...prev, { role: 'system', content: 'Error: Failed to get response from LLM.' }]);
    } finally {
      setIsLoading(false);
      // Persist assistant message once done
      if (assistantContent && currentThreadId) {
        await saveMessage(currentThreadId, 'assistant', assistantContent);
        fetchThreads(); // Refresh sidebar
      }
    }
  };

  const handleStop = () => {
    abortControllerRef.current?.abort();
    abortControllerRef.current = null;
    setIsLoading(false);
  };

  const handleSuggestionClick = (text: string) => {
    setInput(text);
  };

  const formatModelName = (name: string | null) => {
    if (!name) return name;
    if (name.includes('models--')) {
      return name.split('models--').pop()?.split('/')[0].split('--').pop() || name;
    }
    return name.split('/').pop() || name;
  };

  return (
    <Box style={{ display: 'flex', height: '100%', margin: '-16px' }}>
      {/* Sidebar - Light Style */}
      <Box
        style={{
          width: 260,
          backgroundColor: 'white',
          borderRight: '1px solid #dee2e6',
          display: 'flex',
          flexDirection: 'column',
          padding: 12,
          color: '#1a1a1a'
        }}
      >
        <Stack gap="xs">
          <Group justify="space-between" mb="xs">
            <UnstyledButton
              style={{
                flex: 1,
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                padding: '8px 12px',
                borderRadius: 8,
                transition: 'background-color 0.2s ease',
                '&:hover': { backgroundColor: '#f1f3f5' }
              }}
              onClick={handleNewChat}
            >
              <Box style={{ width: 24, height: 24, borderRadius: '50%', backgroundColor: '#f1f3f5', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                <img src="https://raw.githubusercontent.com/ollama/ollama/main/docs/ollama.png" alt="logo" style={{ width: 16, height: 16 }} />
              </Box>
              <Text size="sm" fw={500}>New Chat</Text>
              <IconEdit size={16} style={{ marginLeft: 'auto' }} />
            </UnstyledButton>
          </Group>


          <TextInput
            mt="md"
            placeholder="Search"
            leftSection={<IconSearch size={16} />}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.currentTarget.value)}
            styles={{
              input: {
                backgroundColor: '#f1f3f5',
                border: 'none',
                color: '#1a1a1a',
                '&::placeholder': { color: '#8e8e8e' },
                '&:focus': { border: '1px solid #ced4da' }
              }
            }}
          />
        </Stack>

        <ScrollArea style={{ flex: 1, marginTop: 20 }}>
          <Stack gap={4}>
            {threads.map((thread) => (
              <UnstyledButton
                key={thread.id}
                p="xs"
                onClick={() => loadThread(thread.id)}
                style={{
                  borderRadius: 8,
                  backgroundColor: threadId === thread.id ? '#f1f3f5' : 'transparent',
                  width: '100%',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                }}
              >
                <Group gap="xs" wrap="nowrap" style={{ flex: 1, overflow: 'hidden' }}>
                  <IconMessage size={16} style={{ flexShrink: 0, opacity: 0.6 }} />
                  {loadingTitleIds.has(thread.id) ? (
                    <Loader size={12} color="gray" />
                  ) : (
                    <Text size="xs" truncate style={{ flex: 1 }}>
                      {thread.title || thread.id}
                    </Text>
                  )}
                </Group>
              </UnstyledButton>
            ))}
          </Stack>
        </ScrollArea>
      </Box>

      {/* Main Chat Area - Clean Light Style */}
      <Box style={{ flex: 1, display: 'flex', flexDirection: 'column', position: 'relative', backgroundColor: 'white' }}>
        {/* Model Header */}
        <Box p="xs" style={{ borderBottom: '1px solid #f2f2f2' }}>
          <Group justify="space-between">
            <Group gap="xs">
              <Text size="sm" c="dimmed">Ollama Web UI</Text>
            </Group>
            <Group gap="xs">
              <Button
                variant="subtle"
                color="gray"
                size="compact-xs"
                rightSection={<IconPlus size={12} />}
              >
                {formatModelName(modelName) || 'Select a model'}
              </Button>
              <ActionIcon variant="subtle" size="sm" onClick={() => navigate('/settings')}><IconSettings size={18} /></ActionIcon>
            </Group>
          </Group>
        </Box>

        {/* Messages */}
        <ScrollArea style={{ flex: 1 }} viewportRef={viewport}>
          <Container size="md" pt={messages.length === 0 && !threadId ? 100 : 40} pb={140}>
            {messages.length === 0 && !threadId ? (
              <Stack align="center" gap="xl">
                <Box style={{ opacity: 1 }}>
                  <img src="https://raw.githubusercontent.com/ollama/ollama/main/docs/ollama.png" alt="Ollama" style={{ width: 80, height: 80 }} />
                </Box>
                <Text size="xl" fw={600} style={{ fontSize: rem(28) }}>How can I help you today?</Text>

                <Box style={{ width: '100%', maxWidth: 700 }}>
                  <Group gap="md" grow>
                    <Paper withBorder p="md" radius="md" style={{ cursor: 'pointer' }} onClick={() => handleSuggestionClick("Tell me a fun fact about the Roman Empire")}>
                      <Text fw={600} size="sm">Tell me a fun fact</Text>
                      <Text size="xs" c="dimmed">about the Roman Empire</Text>
                    </Paper>
                    <Paper withBorder p="md" radius="md" style={{ cursor: 'pointer' }} onClick={() => handleSuggestionClick("Show me a code snippet of a website's sticky header")}>
                      <Text fw={600} size="sm">Show me a code snippet</Text>
                      <Text size="xs" c="dimmed">of a website's sticky header</Text>
                    </Paper>
                  </Group>
                  <Group gap="md" grow mt="md">
                    <Paper withBorder p="md" radius="md" style={{ cursor: 'pointer' }} onClick={() => handleSuggestionClick("Help me study vocabulary for a college entrance exam")}>
                      <Text fw={600} size="sm">Help me study</Text>
                      <Text size="xs" c="dimmed">vocabulary for a college entrance exam</Text>
                    </Paper>
                    <Paper withBorder p="md" radius="md" style={{ cursor: 'pointer' }} onClick={() => handleSuggestionClick("Give me ideas for what to do with my kids' art")}>
                      <Text fw={600} size="sm">Give me ideas</Text>
                      <Text size="xs" c="dimmed">for what to do with my kids' art</Text>
                    </Paper>
                  </Group>
                </Box>
              </Stack>
            ) : (
              <Stack gap="xl">
                {messages.map((m, i) => (
                  <Group
                    key={i}
                    align="flex-start"
                    wrap="nowrap"
                    gap="sm"
                    style={{
                      flexDirection: m.role === 'user' ? 'row-reverse' : 'row',
                      alignSelf: m.role === 'user' ? 'flex-end' : 'flex-start',
                      maxWidth: '85%',
                    }}
                  >
                    <Avatar
                      size="md"
                      radius="xl"
                      color={m.role === 'user' ? 'blue' : 'gray'}
                      bg={m.role === 'user' ? 'blue.1' : 'gray.1'}
                      style={{ flexShrink: 0 }}
                    >
                      {m.role === 'user' ? (
                        <Text size="xs" fw={700} color="blue">P</Text>
                      ) : (
                        <img src="https://raw.githubusercontent.com/ollama/ollama/main/docs/ollama.png" alt="AI" style={{ width: 24 }} />
                      )}
                    </Avatar>
                    <Stack gap={4} style={{ alignItems: m.role === 'user' ? 'flex-end' : 'flex-start' }}>
                      <Text fw={700} size="xs" c="dimmed" style={{ paddingLeft: m.role === 'user' ? 0 : 4, paddingRight: m.role === 'user' ? 4 : 0 }}>
                        {m.role === 'user' ? 'You' : 'Assistant'}
                      </Text>
                      <Paper
                        withBorder
                        px="md"
                        py="xs"
                        radius="lg"
                        style={{
                          backgroundColor: m.role === 'user' ? '#ffffff' : '#f8f9fa',
                          borderColor: m.role === 'user' ? '#dee2e6' : '#e9ecef',
                          borderTopRightRadius: m.role === 'user' ? 4 : 20,
                          borderTopLeftRadius: m.role === 'user' ? 20 : 4,
                          boxShadow: '0 1px 2px rgba(0,0,0,0.05)',
                        }}
                      >
                        <Box style={{ lineHeight: '1.6', fontSize: rem(15), color: '#1a1a1a' }}>
                          <ReactMarkdown remarkPlugins={[remarkGfm]}>
                            {m.content}
                          </ReactMarkdown>
                        </Box>
                      </Paper>
                    </Stack>
                  </Group>
                ))}
                {isLoading && (
                  <Group align="flex-start" wrap="nowrap" gap="sm" style={{ maxWidth: '85%' }}>
                    <Avatar size="md" radius="xl" color="gray" bg="gray.1" style={{ flexShrink: 0 }}>
                      <img src="https://raw.githubusercontent.com/ollama/ollama/main/docs/ollama.png" alt="AI" style={{ width: 24 }} />
                    </Avatar>
                    <Stack gap={4}>
                      <Text fw={700} size="xs" c="dimmed" style={{ paddingLeft: 4 }}>Assistant</Text>
                      <Paper
                        withBorder
                        px="md"
                        py="xs"
                        radius="lg"
                        style={{
                          backgroundColor: '#f8f9fa',
                          borderColor: '#e9ecef',
                          borderTopLeftRadius: 4,
                          boxShadow: '0 1px 2px rgba(0,0,0,0.05)',
                        }}
                      >
                        <Box style={{ height: 24, display: 'flex', alignItems: 'center' }}>
                          <Text size="sm" c="dimmed">...</Text>
                        </Box>
                      </Paper>
                    </Stack>
                  </Group>
                )}
              </Stack>
            )}
          </Container>
        </ScrollArea>

        {/* Input Bar */}
        <Box
          p="md"
          style={{
            position: 'absolute',
            bottom: 0,
            left: 0,
            right: 0,
            background: 'linear-gradient(rgba(255,255,255,0), rgba(255,255,255,1) 40%)',
            paddingBottom: 24
          }}
        >
          <Container size="md" style={{ maxWidth: 760 }}>
            <Paper withBorder shadow="md" radius="lg" p={6}>
              <form onSubmit={handleSubmit}>
                <Group gap={4} align="flex-end">
                  <ActionIcon variant="subtle" size="lg" radius="md" color="gray">
                    <IconPaperclip size={20} />
                  </ActionIcon>
                  <Textarea
                    variant="unstyled"
                    placeholder="Send a message"
                    autoFocus
                    autosize
                    minRows={1}
                    maxRows={10}
                    style={{ flex: 1 }}
                    styles={{ input: { paddingLeft: 8, paddingRight: 8, fontSize: rem(16), lineHeight: '1.5' } }}
                    value={input}
                    onChange={(e) => setInput(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' && !e.shiftKey) {
                        e.preventDefault();
                        handleSubmit();
                      }
                    }}
                  />
                  <Group gap={4}>
                    <ActionIcon variant="subtle" size="lg" radius="md" color="gray">
                      <IconMicrophone size={20} />
                    </ActionIcon>
                    {isLoading ? (
                      <ActionIcon
                        onClick={handleStop}
                        variant="filled"
                        size="lg"
                        radius="md"
                        color="dark"
                      >
                        <IconSquare size={20} />
                      </ActionIcon>
                    ) : (
                      <ActionIcon
                        onClick={handleSubmit}
                        variant="filled"
                        size="lg"
                        radius="md"
                        color="dark"
                        disabled={!input.trim()}
                      >
                        <IconArrowUp size={20} />
                      </ActionIcon>
                    )}
                  </Group>
                </Group>
              </form>
            </Paper>
            <Text size="xs" c="dimmed" ta="center" mt={8}>
              LLMs can make mistakes. Verify important information.
            </Text>
          </Container>
        </Box>
      </Box>
    </Box>
  );
}