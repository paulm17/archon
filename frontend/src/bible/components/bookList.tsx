import { useEffect, useState } from 'react';
import { Box, Button, Flex, Group, Loader, Popover, ScrollArea, Stack, Text } from '@mantine/core';
import { usePage } from '../pageContext';

interface BookListProps {
  fetchChapter: (bookCode: string, chapter: number, versionOverride?: string) => void;
}

export function BookList({ fetchChapter }: BookListProps) {
  const { booksGrouped, loadingBooks, selectedBook, setSelectedBook, version } = usePage();

  const [bookModalOpen, setBookModalOpen] = useState(false);
  const [selectedChapter, setSelectedChapter] = useState<number | null>(null);

  // Extract categories directly from booksGrouped
  const oldTestament = booksGrouped?.categories?.find((cat) => cat.name === 'Old Testament');
  const newTestament = booksGrouped?.categories?.find((cat) => cat.name === 'New Testament');
  const apocrypha = booksGrouped?.categories?.find((cat) => cat.name === 'Apocrypha');

  // Set default selected book to the first available book when booksGrouped loads
  useEffect(() => {
    const firstBook =
      oldTestament?.books?.[0] ?? newTestament?.books?.[0] ?? apocrypha?.books?.[0] ?? null;

    if (booksGrouped && !selectedBook && firstBook) {
      setSelectedBook({ id: firstBook.id, code: firstBook.code });
    }
  }, [booksGrouped, selectedBook, oldTestament, newTestament, apocrypha, setSelectedBook]);

  function handleChapterClick(chapter: number) {
    if (selectedBook) {
      setBookModalOpen(false);
      setSelectedChapter(chapter);
      fetchChapter(selectedBook.code, chapter, version);
    }
  }

  // Get chapters for selected book
  const selectedBookData = booksGrouped?.categories
    ?.flatMap((cat) => cat.books)
    .find((book) => book.id === selectedBook?.id);

  return (
    <Popover
      opened={bookModalOpen}
      onChange={setBookModalOpen}
      width={800}
      position="bottom-start"
      shadow="md"
    >
      <Popover.Target>
        <Button onClick={() => setBookModalOpen(true)}>Books</Button>
      </Popover.Target>
      <Popover.Dropdown>
        <Flex gap={8} style={{ height: '500px' }}>
          {/* Old Testament Column */}
          <ScrollArea style={{ flexBasis: '30%', borderRight: '1px solid #dee2e6' }} scrollbars="y">
            {loadingBooks ? (
              <Loader size="sm" />
            ) : (
              <Stack gap={4} p={6} style={{ fontSize: '14px', lineHeight: 1.25 }}>
                <Text fw={700} size="sm" mb="xs" tt="uppercase">
                  OLD TESTAMENT
                </Text>
                {oldTestament?.books.map((book) => (
                  <Box key={book.id}>
                    <Text
                      onClick={() => {
                        setSelectedChapter(null);
                        setSelectedBook({ id: book.id, code: book.code });
                      }}
                      style={{
                        cursor: 'pointer',
                        padding: '2px 4px',
                        color: selectedBook?.id === book.id ? '#ff6b6b' : 'inherit',
                      }}
                    >
                      {book.name}
                      {selectedBook?.id === book.id ? ' ›' : ''}
                    </Text>
                  </Box>
                ))}
              </Stack>
            )}
          </ScrollArea>

          {/* New Testament Column */}
          <ScrollArea style={{ flexBasis: '30%', borderRight: '1px solid #dee2e6' }} scrollbars="y">
            {loadingBooks ? (
              <Loader size="sm" />
            ) : (
              <Stack gap={4} p={6} style={{ fontSize: '14px', lineHeight: 1.25 }}>
                <Text fw={700} size="sm" mb="xs" tt="uppercase">
                  NEW TESTAMENT
                </Text>
                {newTestament?.books.map((book) => (
                  <Box key={book.id}>
                    <Text
                      onClick={() => {
                        setSelectedChapter(null);
                        setSelectedBook({ id: book.id, code: book.code });
                      }}
                      style={{
                        cursor: 'pointer',
                        padding: '2px 4px',
                        color: selectedBook?.id === book.id ? '#ff6b6b' : 'inherit',
                      }}
                    >
                      {book.name}
                      {selectedBook?.id === book.id ? ' ›' : ''}
                    </Text>
                  </Box>
                ))}
                {apocrypha && apocrypha.books.length > 0 && (
                  <>
                    <Text fw={700} size="sm" mb="xs" tt="uppercase" mt="md">
                      APOCRYPHA
                    </Text>
                    {apocrypha.books.map((book) => (
                      <Box key={book.id}>
                        <Text
                          onClick={() => {
                            setSelectedChapter(null);
                            setSelectedBook({ id: book.id, code: book.code });
                          }}
                          style={{
                            cursor: 'pointer',
                            padding: '2px 4px',
                            color: selectedBook?.id === book.id ? '#ff6b6b' : 'inherit',
                          }}
                        >
                          {book.name}
                          {selectedBook?.id === book.id ? ' ›' : ''}
                        </Text>
                      </Box>
                    ))}
                  </>
                )}
              </Stack>
            )}
          </ScrollArea>

          {/* Chapters Column */}
          <ScrollArea style={{ flexBasis: '40%' }} scrollbars="y">
            <Stack gap={4} py={6} px={4} style={{ fontSize: '14px', lineHeight: 1.25 }}>
              {selectedBook && selectedBookData ? (
                <>
                  <Text fw={700} size="sm" mb="xs" tt="uppercase">
                    {selectedBookData.name.toUpperCase()}
                  </Text>
                  <Stack gap={2} style={{ fontSize: '14px', lineHeight: 1.25 }}>
                    {selectedBookData.chapters.map((ch) => (
                      <Box
                        key={ch.number}
                        onClick={() => handleChapterClick(ch.number)}
                        style={{
                          cursor: 'pointer',
                          padding: '2px 6px',
                          backgroundColor:
                            selectedChapter === ch.number ? '#ff6b6b' : 'transparent',
                          color: selectedChapter === ch.number ? 'white' : 'inherit',
                        }}
                      >
                        <Group gap={4}>
                          <Text>{ch.number}</Text>
                          <Text truncate="end" style={{ width: '90%' }}>
                            {ch.heading}
                          </Text>
                        </Group>
                      </Box>
                    ))}
                  </Stack>
                </>
              ) : (
                <Text c="dimmed" ta="center" mt="xl">
                  Select a book to see chapters
                </Text>
              )}
            </Stack>
          </ScrollArea>
        </Flex>
      </Popover.Dropdown>
    </Popover>
  );
}
