import { useState } from 'react';
import { Button, Container, Group, Table, TextInput, Stack, Alert } from '@mantine/core';
import { scriptureClient } from '../lib/client';

type TranslationResult = {
  translationName: string;
  text: string;
};

type BookInfo = {
  code: string;
  name: string;
};

// ============================================================================
// PARSING LOGIC (Duplicated from getPassage.ts)
// ============================================================================

function matchBook(abbrev: string, books: BookInfo[]): string | null {
  const normalized = abbrev.toLowerCase().trim();

  // Check book.code
  for (const b of books) {
    if (b.code.toLowerCase() === normalized) return b.code;
  }

  // Check full book.name exact
  for (const b of books) {
    if (b.name.toLowerCase() === normalized) return b.code;
  }

  // Check startsWith name
  for (const b of books) {
    if (b.name.toLowerCase().startsWith(normalized)) return b.code;
  }

  // Check contains in name
  for (const b of books) {
    if (b.name.toLowerCase().includes(normalized)) return b.code;
  }

  return null;
}

function parseInput(input: string, books: BookInfo[]): { bookCode: string; chapter: number; verse: number } | null {
  const cleaned = input.trim();
  
  const match = cleaned.match(/^([1-3]?[A-Za-z]+)\s+(\d+)(?::(\d+))?$/);
  
  if (!match) {
    return null;
  }

  const abbrev = match[1];
  const chapterStr = match[2];
  const verseStr = match[3];

  if (!verseStr) {
    return null;
  }

  const bookCode = matchBook(abbrev, books);
  if (!bookCode) {
    return null;
  }

  return {
    bookCode,
    chapter: parseInt(chapterStr, 10),
    verse: parseInt(verseStr, 10),
  };
}

export function TranslationPage() {
  const [search, setSearch] = useState('');
  const [results, setResults] = useState<TranslationResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hasSearched, setHasSearched] = useState(false);

  const getVerseInAllTranslations = async (search: string): Promise<TranslationResult[]> => {
      // 1. Fetch all books for name matching
      const result = await scriptureClient.query(`
        query GetBooksForTranslationLookup {
          queryBook {
            id
            code
            nameEn
          }
        }
      `, {}).toPromise();

      if (result.error) {
        throw result.error;
      }

      const queryBook = result.data?.queryBook;
    
      const booksFromDb = queryBook || [];
    
      const books: BookInfo[] = booksFromDb.map((book: any) => ({
        code: book.code || '',
        name: book.nameEn || '', 
      }));
    
      // 2. Parse the input string
      const parsed = parseInput(search, books);
      
      if (!parsed) {
        return [];
      }
    
      const { bookCode, chapter, verse } = parsed;
    
      console.log(`Searching for ${bookCode} ${chapter}:${verse} in all translations`);
    
      // 3. Fetch verses from all translations
      const result2 = await scriptureClient.query(`
        query GetVerseInAllTranslations($bookCode: String!, $chapter: Int!, $verse: Int!) {
          queryVerse(filter: {
            chunkType: { eq: "single_verse" }
            number: { eq: $verse }
            chapter: {
              number: { eq: $chapter }
              bookTranslation: {
                book: {
                  code: { eq: $bookCode }
                }
              }
            }
          }) {
            id
            verseContents {
              id
              text
              type
              translation {
                id
                name
              }
            }
          }
        }
      `, { bookCode, chapter, verse }).toPromise();

      if (result2.error) {
        throw result2.error;
      }

      const queryVerse = result2.data?.queryVerse;
      
      const verses = queryVerse || [];
    
      // 4. Transform results
      const results: TranslationResult[] = [];
    
      for (const v of verses) {
        const contents = v.verseContents || [];
        for (const content of contents) {
          if (content?.type === 'text' && content?.text && content?.translation?.name) {
            results.push({
              translationName: content.translation.name,
              text: content.text,
            });
          }
        }
      }
    
      // Sort alphabetically by translation name
      results.sort((a, b) => a.translationName.localeCompare(b.translationName));
    
      return results;
  }

  const handleSearch = async () => {
    if (!search.trim()) return;

    setLoading(true);
    setError(null);
    setResults([]);
    setHasSearched(true);

    try {
      const data = await getVerseInAllTranslations(search);
      setResults(data);
      if (data.length === 0) {
        setError('No results found. Please ensure format is like "Matthew 1:1"');
      }
    } catch (err) {
      console.error(err);
      setError('An error occurred while fetching translations.');
    } finally {
      setLoading(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      handleSearch();
    }
  };

  const rows = results.map((row) => (
    <Table.Tr key={row.translationName}>
      <Table.Td style={{ width: '150px', fontWeight: 600 }}>{row.translationName}</Table.Td>
      <Table.Td>{row.text}</Table.Td>
    </Table.Tr>
  ));

  return (
    <Container size="100%" px="lg">
      <Stack gap="lg">
        <Group align="flex-end">
          <TextInput
            label="Verse Reference"
            placeholder="e.g. Matthew 1:1"
            value={search}
            onChange={(event) => setSearch(event.currentTarget.value)}
            onKeyDown={handleKeyDown}
            style={{ flex: 1 }}
          />
          <Button onClick={handleSearch} loading={loading}>
            Search
          </Button>
        </Group>

        {error && (
            <Alert color="red" title="Error">
                {error}
            </Alert>
        )}

        {hasSearched && results.length > 0 && (
          <Table striped highlightOnHover>
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Translation</Table.Th>
                <Table.Th>Verse</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>{rows}</Table.Tbody>
          </Table>
        )}
      </Stack>
    </Container>
  );
}
