import { IconLanguage } from '@tabler/icons-react';
import {
  ActionIcon,
  Box,
  Button,
  Flex,
  Group,
  Select,
  Stack,
  TextInput,
  Tooltip,
} from '@mantine/core';
import { usePage } from '../pageContext';
import { usePanes } from '../paneContext';
import type { GospelSelection } from '../types';
import { BookList } from './bookList';

interface HeaderProps {
  handleSubmit: () => void;
  handleReset: () => void;
  fetchChapter: (bookCode: string, chapter: number) => Promise<void>;
}

export function Header({ handleSubmit, handleReset, fetchChapter }: HeaderProps) {
  const { search, setSearch, version, setVersion, gospel, setGospel, translations, hasSearched } =
    usePage();
  const { panes, getActiveTab, toggleLanguagePane, languagePaneLanguage } = usePanes();

  // Filter out original language translations and hide interlinear in canonical mode
  const filteredTranslations = translations.filter((t) => {
    if (['WLC', 'SBLGNT', 'TR1894'].includes(t.code)) {
      return false;
    }

    if (gospel === 'canonical') {
      const lowerCode = t.code.toLowerCase();
      const lowerName = t.name.toLowerCase();
      if (lowerCode.includes('interlinear') || lowerName.includes('interlinear')) {
        return false;
      }
    }

    return true;
  });

  // Determine language button based on first pane's active results
  const firstPane = panes[0];
  const firstPaneTab = getActiveTab(firstPane);
  const firstPaneResult = firstPaneTab?.results?.[0];

  let languageToShow: 'he' | 'gr' | null = null;
  if (hasSearched && firstPaneResult?.isWholeChapter) {
    // Determine language based on book code
    const bookCode = firstPaneResult.reference.bookCode;
    // Simple heuristic: NT books start with Matt, Mark, Luke, John, Acts, Rom, 1Cor, 2Cor, Gal, Eph, Phil, Col, 1Thess, 2Thess, 1Tim, 2Tim, Titus, Phlm, Heb, Jas, 1Pet, 2Pet, 1John, 2John, 3John, Jude, Rev
    const ntBooks = [
      'Matt',
      'Mark',
      'Luke',
      'John',
      'Acts',
      'Rom',
      '1Cor',
      '2Cor',
      'Gal',
      'Eph',
      'Phil',
      'Col',
      '1Thess',
      '2Thess',
      '1Tim',
      '2Tim',
      'Titus',
      'Phlm',
      'Heb',
      'Jas',
      '1Pet',
      '2Pet',
      '1John',
      '2John',
      '3John',
      'Jude',
      'Rev',
    ];
    languageToShow = ntBooks.includes(bookCode) ? 'gr' : 'he';
  }

  const showLanguageButton = languageToShow !== null;
  const isLanguagePaneOpen = languagePaneLanguage !== null;

  return (
    <Stack>
      <Group align="end" gap="sm">
        <TextInput
          label="Passage"
          placeholder="Luke 1"
          value={search}
          onChange={(e) => setSearch(e.currentTarget.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleSubmit()}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          name="passage-search"
          data-lpignore="true"
          data-form-type="other"
          disabled={hasSearched}
          flex={{ base: '1 1 100%', md: 1 }}
        />
        <Box w={{ base: '100%', md: 220 }}>
          <Select
            label="Gospel"
            value={gospel}
            onChange={(val) => val && setGospel(val as GospelSelection)}
            data={[
              { value: 'canonical', label: 'Canonical' },
              { value: 'gospel-of-thomas', label: 'Gospel of Thomas' },
            ]}
            disabled={hasSearched}
          />
        </Box>
        <Box w={{ base: '100%', md: 325 }}>
          <Select
            label="Translation"
            value={version}
            onChange={(val) => setVersion(val!)}
            data={filteredTranslations.map((t) => ({ value: t.code, label: t.name }))}
            searchable
            disabled={hasSearched}
          />
        </Box>
      </Group>
      <Flex justify="space-between">
        <BookList fetchChapter={fetchChapter} />
        <Group gap="xs">
          {showLanguageButton && (
            <Tooltip label={languageToShow === 'he' ? 'Hebrew' : 'Greek'}>
              <ActionIcon
                variant={isLanguagePaneOpen ? 'filled' : 'light'}
                color="violet"
                onClick={() => toggleLanguagePane(languageToShow!)}
                size="lg"
              >
                <IconLanguage size={18} />
              </ActionIcon>
            </Tooltip>
          )}
          {!hasSearched ? (
            <Button onClick={handleSubmit} disabled={!search.trim()}>
              Search
            </Button>
          ) : (
            <Button color="red" onClick={handleReset}>
              Reset
            </Button>
          )}
        </Group>
      </Flex>
    </Stack>
  );
}
