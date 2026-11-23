import { useEffect } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Container, Flex, Text } from '@mantine/core';
import { Header } from './components/header';
import { Pane } from './components/pane';
import { usePage } from './pageContext';
import { PaneProvider, usePanes } from './paneContext';

import { getTranslations, getBooksGrouped } from './api';
import { PageProvider } from './pageContext';

export default function BiblePage() {
  return (
    <PageProvider>
      <BiblePageContent />
    </PageProvider>
  );
}

function BiblePageContent() {
  const [searchParams] = useSearchParams();

  // Use page context
  const {
    search,
    setSearch,
    version,
    setVersion,
    gospel,
    setTranslations,
    setBooksGrouped,
    setLoadingBooks,
  } = usePage();

  const translation = searchParams.get('version') || version;

  // Initialize from URL params
  useEffect(() => {
    if (search) {
      setSearch(search);
    }
    if (translation) {
      setVersion(translation);
    }
  }, [search, translation, setSearch, setVersion]);

//   Fetch translations for selected gospel (cached per gospel in api.ts)
  useEffect(() => {
    async function fetchTranslations() {
      const data = await getTranslations(gospel);
      setTranslations(data);
      if (data.length > 0 && !data.some((t) => t.code === version)) {
        setVersion(data[0].code as any);
      } else if (data.length === 0) {
        setVersion('' as any);
      }
    }
    fetchTranslations();
  }, [gospel, setTranslations, setVersion, version]);

//   Fetch canonical books only for canonical gospel mode
  useEffect(() => {
    if (gospel !== 'canonical') {
      setBooksGrouped(null);
      setLoadingBooks(false);
      return;
    }

    async function fetchBooks() {
      setLoadingBooks(true);
      const data = await getBooksGrouped(version);
      if (data !== null) {
        setBooksGrouped(data);
      }
      setLoadingBooks(false);
    }
    fetchBooks();
  }, [gospel, version, setBooksGrouped, setLoadingBooks]);

  return (
    <PaneProvider translation={translation} gospel={gospel}>
      <PassagePageContent />
    </PaneProvider>
  );
}

function PassagePageContent() {
  const { panes, handleSubmit, fetchChapter, getActiveTab, resetPanes } = usePanes();
  const { search, version, gospel, setSearch, setHasSearched } = usePage();

  const handleSearchClick = () => {
    const searchValue = typeof search === 'string' ? search : String(search || '');
    const versionValue = typeof version === 'string' ? version : String(version || '');
    handleSubmit(searchValue, versionValue, gospel);
    setHasSearched(true);
  };

  const handleReset = () => {
    setSearch('');
    setHasSearched(false);
    resetPanes();
  };

  return (
    <Container size="100%" px="lg" py="lg">
      <Header
        handleSubmit={handleSearchClick}
        handleReset={handleReset}
        fetchChapter={fetchChapter}
      />

      <Flex gap="md" wrap="wrap" align="stretch">
        {panes.map((pane, index) => {
          const activeTab = getActiveTab(pane);
          if (!activeTab) return null;

          return <Pane key={pane.id} pane={pane} paneIndex={index} />;
        })}
      </Flex>

      {panes.every((p) => (getActiveTab(p)?.results.length ?? 0) === 0) &&
        !panes.some((p) => getActiveTab(p)?.loading) && (
          <Text c="dimmed" ta="center" mt="xl">
            No verses loaded
          </Text>
        )}
    </Container>
  );
}
