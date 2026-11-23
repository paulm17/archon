import { useRef } from 'react';
import { IconPlus, IconX } from '@tabler/icons-react';
import {
  ActionIcon,
  Box,
  Flex,
  Group,
  LoadingOverlay,
  Pagination,
  Paper,
  ScrollArea,
  Select,
  Stack,
  Tabs,
  Text,
} from '@mantine/core';
import { usePage } from '../pageContext';
import { usePanes } from '../paneContext';
import type { PaneObserverState, Pane as PaneType } from '../types';
import { Footer } from './footer';
import { PassageSection } from './passageSection';

interface PaneProps {
  pane: PaneType;
  paneIndex: number;
}

export function Pane({ pane, paneIndex }: PaneProps) {
  const { translations, booksGrouped, getFullBookName, gospel } = usePage();
  const {
    panes,
    activeIndexes,
    updateActiveIndex,
    addPane,
    removePane,
    setActiveTab,
    closeTab,
    changePaneTranslation,
    fetchChapter,
    getActiveTab,
    shouldShowPagination,
    toggleLanguagePane,
    loadingPaneIds,
    isChapterLoading,
  } = usePanes();

  const paneObserversRef = useRef<Map<number, PaneObserverState>>(new Map());

  const activeTab = getActiveTab(pane);
  if (!activeTab) return null;

  const key = `${pane.id}-${activeTab.id}`;
  const activeIndex = activeIndexes[key] ?? 0;
  const activeResult = activeTab.results[activeIndex] ?? activeTab.results[0] ?? null;
  if (!activeResult && !activeTab.loading) return null;

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

  // Count only regular panes (exclude language pane with ID 999)
  const regularPanes = panes.filter((p) => p.id !== 999);
  const isLanguagePane = pane.id === 999;
  const isMainPane = pane.id === 1;

  const ensurePaneObserver = (paneId: number, rootEl: HTMLElement | null) => {
    let state = paneObserversRef.current.get(paneId);

    const thresholds = [0, 0.1, 0.25, 0.5, 0.65, 0.75, 0.9, 1];

    const onIntersect: IntersectionObserverCallback = (ioEntries) => {
      const s = paneObserversRef.current.get(paneId);
      if (!s) return;

      for (const entry of ioEntries) {
        const el = entry.target as HTMLElement;
        const idx = parseInt(el.getAttribute('data-section-index') || '0', 10);
        if (!isNaN(idx)) {
          s.entries.set(idx, { el, ratio: entry.intersectionRatio });
        }
      }

      if (s.debounceTimer != null) {
        clearTimeout(s.debounceTimer);
        s.debounceTimer = null;
      }

      s.debounceTimer = window.setTimeout(() => {
        const st = paneObserversRef.current.get(paneId);
        if (!st) return;

        st.debounceTimer = null;

        let bestIdx = -1;
        let bestRatio = -1;

        for (const [idx, { ratio }] of st.entries.entries()) {
          if (ratio > bestRatio) {
            bestRatio = ratio;
            bestIdx = idx;
          }
        }

        if (bestIdx >= 0 && st.lastTopIndex !== bestIdx) {
          st.lastTopIndex = bestIdx;
          const pn = panes.find((p) => p.id === paneId);
          const atab = pn ? getActiveTab(pn) : null;
          if (atab) {
            updateActiveIndex(paneId, atab.id, bestIdx);
          }
        }
      }, 50);
    };

    if (!state) {
      state = {
        root: rootEl,
        observer: new IntersectionObserver(onIntersect, {
          root: rootEl || undefined,
          threshold: thresholds,
          rootMargin: '0px 0px -20% 0px',
        }),
        entries: new Map(),
        rafId: null,
        debounceTimer: null,
        lastTopIndex: null,
      };
      paneObserversRef.current.set(paneId, state);
    } else {
      if (state.root !== rootEl) {
        state.observer.disconnect();
        state.root = rootEl;
        state.observer = new IntersectionObserver(onIntersect, {
          root: rootEl || undefined,
          threshold: thresholds,
          rootMargin: '0px 0px -20% 0px',
        });
        for (const { el } of state.entries.values()) {
          state.observer.observe(el);
        }
      }
    }
  };

  const showPagination = shouldShowPagination(activeTab.searchQuery, activeTab.results);

  // Check if this specific pane is loading
  const isPaneLoading = loadingPaneIds.has(pane.id);

  let footnotes = 0;
  let crossrefs = 0;

  if (activeResult && activeResult.footnotes) {
    footnotes = activeResult.footnotes.length;
  }

  if (activeResult && activeResult.crossrefs) {
    crossrefs = activeResult.crossrefs.length;
  }

  // Translation selector component logic
  const renderTranslationSelector = () => {
    if (isLanguagePane) {
      // Hebrew pane shows only WLC as text
      if (pane.version === 'WLC') {
        return (
          <Text size="sm" fw={500}>
            WLC
          </Text>
        );
      }
      // Greek pane shows dropdown for SBLGNT/TR1894
      return (
        <Select
          value={pane.version}
          onChange={(val) => val && changePaneTranslation(pane.id, val)}
          data={[
            { value: 'SBLGNT', label: 'SBLGNT' },
            { value: 'TR1894', label: 'TR1894' },
          ]}
          size="xs"
          w={100}
        />
      );
    }

    // Regular panes show filtered translations (excluding original languages)
    if (!isMainPane) {
      return (
        <Select
          value={pane.version}
          onChange={(val) => val && changePaneTranslation(pane.id, val)}
          data={filteredTranslations.map((t) => ({ value: t.code, label: t.code }))}
          size="xs"
          w={100}
          searchable
        />
      );
    }

    return null;
  };

  // Pane controls component logic
  const renderPaneControls = () => {
    if (isLanguagePane) {
      return (
        <ActionIcon
          variant="light"
          color="red"
          onClick={() => toggleLanguagePane(pane.version === 'WLC' ? 'he' : 'gr')}
          size="lg"
        >
          <IconX size={18} />
        </ActionIcon>
      );
    }

    return (
      <ActionIcon.Group>
        <ActionIcon
          variant="light"
          color="blue"
          onClick={addPane}
          size="lg"
          disabled={regularPanes.length >= 3}
        >
          <IconPlus size={18} />
        </ActionIcon>
        {!isMainPane && (
          <ActionIcon variant="light" color="red" onClick={() => removePane(pane.id)} size="lg">
            <IconX size={18} />
          </ActionIcon>
        )}
      </ActionIcon.Group>
    );
  };

  return (
    <Box
      style={{
        flex:
          panes.length === 1
            ? '1 1 100%'
            : panes.length === 2
              ? '1 1 calc(50% - 8px)'
              : '1 1 calc(33.333% - 11px)',
      }}
    >
      {activeTab.results.length > 0 ? (
        <Stack gap="md" pos="relative">
          {showPagination && activeResult && (
            <Pagination.Root
              total={activeResult.maxChapter!}
              value={activeResult.reference.chapter}
              onChange={(chapter) => {
                fetchChapter(activeResult.reference.bookCode, chapter);
              }}
              size={50}
              disabled={isChapterLoading}
            >
              {paneIndex === 0 && (
                <Box
                  style={{
                    position: 'absolute',
                    left: '-55px',
                    top: '50%',
                    transform: 'translateY(-50%)',
                    zIndex: 100,
                  }}
                >
                  <Pagination.Previous disabled={isChapterLoading} />
                </Box>
              )}
              {paneIndex === panes.length - 1 && (
                <Box
                  style={{
                    position: 'absolute',
                    right: '-55px',
                    top: '50%',
                    transform: 'translateY(-50%)',
                    zIndex: 100,
                  }}
                >
                  <Pagination.Next disabled={isChapterLoading} />
                </Box>
              )}
            </Pagination.Root>
          )}

          <Paper withBorder mt={!showPagination ? 'md' : 0}>
            <Stack gap="xs">
              <Flex align="center">
                {/* Tabs Section */}
                <Tabs
                  value={String(activeTab.id)}
                  onChange={(value) => value && setActiveTab(pane.id, Number(value))}
                  style={{ flex: 1 }}
                >
                  <Tabs.List>
                    {pane.tabs.open.map((tab) => {
                      const displayLabel =
                        tab.results.length > 0 && tab.results[activeIndex]
                          ? `${getFullBookName(
                              tab.results[activeIndex].reference.bookCode,
                              booksGrouped
                            )} ${tab.results[activeIndex].reference.chapter}${
                              tab.results[activeIndex].reference.verseStart > 0
                                ? ':' + tab.results[activeIndex].reference.verseStart
                                : ''
                            }${
                              tab.results[activeIndex].reference.verseEnd > 0 &&
                              tab.results[activeIndex].reference.verseEnd !==
                                tab.results[activeIndex].reference.verseStart
                                ? '-' + tab.results[activeIndex].reference.verseEnd
                                : ''
                            }`
                          : tab.label || 'Untitled';

                      return (
                        <Tabs.Tab
                          key={tab.id}
                          value={String(tab.id)}
                          rightSection={
                            pane.tabs.open.length > 1 && (
                              <ActionIcon
                                component="span"
                                size="xs"
                                variant="subtle"
                                onClick={(event) => {
                                  event.preventDefault();
                                  event.stopPropagation();
                                  closeTab(pane.id, tab.id);
                                }}
                              >
                                <IconX size={12} />
                              </ActionIcon>
                            )
                          }
                        >
                          <Text size="sm">{displayLabel}</Text>
                        </Tabs.Tab>
                      );
                    })}
                  </Tabs.List>
                </Tabs>

                {/* Controls Section */}
                <Group gap="xs" p={3.75} style={{ borderBottom: '2px solid #dee2e6' }}>
                  {renderTranslationSelector()}
                  {renderPaneControls()}
                </Group>
              </Flex>

              <Box
                pos="relative"
                style={{
                  height: activeTab.footerOpen ? 'calc(100vh - 650px)' : 'calc(100vh - 350px)',
                  overflow: 'hidden',
                }}
              >
                <LoadingOverlay
                  visible={isPaneLoading}
                  zIndex={10}
                  overlayProps={{ radius: 'sm', blur: 2 }}
                />
                <ScrollArea
                  h="100%"
                  scrollbars="y"
                  type="auto"
                  viewportRef={(el) => ensurePaneObserver(pane.id, el || null)}
                >
                  <Box pr="xs">
                    {activeTab.results.length > 0 ? (
                      activeTab.results.map((result, idx) => (
                        <PassageSection
                          key={idx}
                          html={result.html}
                          index={idx}
                          paneId={pane.id}
                          tabId={activeTab.id}
                        />
                      ))
                    ) : (
                      <Text c="dimmed" ta="center" py="xl">
                        Enter a passage to begin
                      </Text>
                    )}
                  </Box>
                </ScrollArea>
              </Box>

              {(footnotes > 0 || crossrefs > 0) && (
                <Footer paneId={pane.id} activeTab={activeTab} activeResult={activeResult} />
              )}
            </Stack>
          </Paper>
        </Stack>
      ) : (
        <Paper p="xl" withBorder>
          <Text c="dimmed" ta="center">
            Enter a passage to begin
          </Text>
        </Paper>
      )}
    </Box>
  );
}
