import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import { getPassage, getPassageForAllPanes } from './api';
import type { GospelSelection, Pane, PaneTab, PassageResult, TranslationCode } from './types';

interface PaneContextState {
  panes: Pane[];
  setPanes: (fn: (prev: Pane[]) => Pane[]) => void;
  activeIndexes: Record<string, number>;
  updateActiveIndex: (paneId: number, tabId: number, index: number) => void;
  addPane: () => void;
  toggleLanguagePane: (languageCode: 'he' | 'gr') => void;
  languagePaneLanguage: 'he' | 'gr' | null;
  removePane: (paneId: number) => void;
  setActiveTab: (paneId: number, tabId: number) => void;
  closeTab: (paneId: number, tabId: number) => void;
  addTab: (paneId: number, searchQuery: string, label: string) => number;
  toggleFooter: (paneId: number, tabId: number, tab: 'footnotes' | 'crossrefs') => void;
  closeFooter: (paneId: number, tabId: number) => void;
  changePaneTranslation: (paneId: number, newVersion: TranslationCode) => void;
  fetchPassageForTab: (
    paneId: number,
    tabId: number,
    searchOverride?: string,
    versionOverride?: string,
    opts?: {
      includeFootnotes?: boolean;
      includeCrossrefs?: boolean;
      label?: string;
      keepContent?: boolean;
    }
  ) => Promise<void>;
  fetchChapter: (bookCode: string, chapter: number, versionOverride?: string) => Promise<void>;
  handleSubmit: (globalSearch: string, globalVersion: string, gospel: GospelSelection) => void;
  handleReferenceClick: (paneId: number, reference: string) => void;
  resetPanes: () => void;
  getActiveTab: (pane: Pane | null | undefined) => PaneTab | null;
  getPane: (paneId: number) => Pane | undefined;
  shouldShowPagination: (searchQuery: string, results: PassageResult[]) => boolean;
  loadingPaneIds: Set<number>;
  isChapterLoading: boolean;
}

const PaneContext = createContext<PaneContextState | null>(null);

interface PaneProviderProps {
  children: ReactNode;
  translation: TranslationCode;
  gospel: GospelSelection;
}

export const PaneProvider = ({ children, translation, gospel }: PaneProviderProps) => {
  const [panes, setPanes] = useState<Pane[]>([]);
  const [nextPaneId, setNextPaneId] = useState(2);
  const [isInitialized, setIsInitialized] = useState(false);
  const [activeIndexes, setActiveIndexes] = useState<Record<string, number>>({});
  const [languagePaneLanguage, setLanguagePaneLanguage] = useState<'he' | 'gr' | null>(null);
  const [loadingPaneIds, setLoadingPaneIds] = useState<Set<number>>(new Set());
  const [isChapterLoading, setIsChapterLoading] = useState(false);

  useEffect(() => {
    if (isInitialized) return;

    const initialTab: PaneTab = {
      id: 1,
      label: 'Start',
      searchQuery: '',
      lifecycle: 'open',
      results: [],
      currentPage: 0,
      loading: false,
      activePassageIndex: 0,
      footerOpen: false,
      footerTab: 'footnotes',
    };

    const firstPane: Pane = {
      id: 1,
      version: translation,
      activeTabId: 1,
      tabs: {
        open: [initialTab],
        available: [],
        closed: [],
      },
    };

    setPanes([firstPane]);
    setIsInitialized(true);
  }, [isInitialized, translation]);

  function getNextTabId(pane: Pane): number {
    const allTabIds = [
      ...pane.tabs.open.map((t) => t.id),
      ...pane.tabs.available.map((t) => t.id),
      ...pane.tabs.closed.map((t) => t.id),
    ];
    return allTabIds.length > 0 ? Math.max(...allTabIds) + 1 : 1;
  }

  function getActiveTab(pane: Pane | null | undefined): PaneTab | null {
    if (!pane) return null;
    return pane.tabs.open.find((tab) => tab.id === pane.activeTabId) ?? null;
  }

  function getPane(paneId: number): Pane | undefined {
    return panes.find((p) => p.id === paneId);
  }

  function shouldShowPagination(searchQuery: string, results: PassageResult[]): boolean {
    const trimmedQuery = searchQuery.trim().replace(/;+$/, '');
    if (trimmedQuery.includes(';')) return false;
    if (results.length > 1) return false;
    const result = results[0];
    if (!result?.isWholeChapter) return false;
    if (!result.maxChapter || result.maxChapter <= 1) return false;
    return true;
  }

  function updateActiveIndex(paneId: number, tabId: number, index: number) {
    setActiveIndexes((prev) => ({
      ...prev,
      [`${paneId}-${tabId}`]: index,
    }));
  }

  async function fetchPassageForTab(
    paneId: number,
    tabId: number,
    searchOverride?: string,
    versionOverride?: string,
    opts?: {
      includeFootnotes?: boolean;
      includeCrossrefs?: boolean;
      label?: string;
      keepContent?: boolean;
    }
  ) {
    const paneNow = panes.find((p) => p.id === paneId);
    const tabNow = paneNow?.tabs.open.find((t) => t.id === tabId);
    const normalizedSearch = String(searchOverride ?? tabNow?.searchQuery ?? '').trim();
    if (!paneNow || !normalizedSearch) return;

    const desiredLabel = String(opts?.label || normalizedSearch || tabNow?.label || 'Untitled');
    const versionToUse = String(versionOverride ?? paneNow.version);

    if (!opts?.keepContent) {
      setPanes((currentPanes) =>
        currentPanes.map((p) =>
          p.id === paneId
            ? {
                ...p,
                version: versionToUse,
                tabs: {
                  ...p.tabs,
                  open: p.tabs.open.map((openTab) =>
                    openTab.id === tabId
                      ? {
                          ...openTab,
                          searchQuery: normalizedSearch,
                          label: desiredLabel,
                          loading: true,
                          lifecycle: 'open' as const,
                        }
                      : openTab
                  ),
                },
              }
            : p
        )
      );
    }

    try {
      const data = await getPassage(normalizedSearch, versionToUse, {
        includeFootnotes: opts?.includeFootnotes,
        includeCrossrefs: opts?.includeCrossrefs,
        gospel,
      });

      setPanes((prev) =>
        prev.map((p) =>
          p.id === paneId
            ? {
                ...p,
                version: versionToUse,
                tabs: {
                  ...p.tabs,
                  open: p.tabs.open.map((openTab) =>
                    openTab.id === tabId
                      ? {
                          ...openTab,
                          results: data,
                          currentPage: 0,
                          loading: false,
                          label: desiredLabel,
                          searchQuery: normalizedSearch,
                          activePassageIndex: 0,
                        }
                      : openTab
                  ),
                },
              }
            : p
        )
      );
    } catch (error) {
      console.error('Error fetching passage:', error);
      setPanes((prev) =>
        prev.map((p) =>
          p.id === paneId
            ? {
                ...p,
                tabs: {
                  ...p.tabs,
                  open: p.tabs.open.map((openTab) =>
                    openTab.id === tabId ? { ...openTab, loading: false } : openTab
                  ),
                },
              }
            : p
        )
      );
    }
  }

  async function fetchChapter(bookCode: string, chapter: number, versionOverride?: string) {
    const newSearch = `${bookCode} ${chapter}`;

    setIsChapterLoading(true);

    const panesToUpdate = panes
      .map((pane) => {
        const activeTab = getActiveTab(pane);
        if (!activeTab) return null;

        return {
          paneId: pane.id,
          tabId: activeTab.id,
          version: versionOverride ?? pane.version,
          searchQuery: newSearch,
          includeFootnotes: true,
          includeCrossrefs: true,
          gospel,
        };
      })
      .filter((item): item is NonNullable<typeof item> => item !== null);

    setLoadingPaneIds(new Set(panesToUpdate.map((p) => p.paneId)));

    try {
      const results = await getPassageForAllPanes(panesToUpdate);

      setPanes((prev) =>
        prev.map((pane) => {
          const result = results.find((r) => r.paneId === pane.id);
          if (!result || result.error) return pane;

          return {
            ...pane,
            tabs: {
              ...pane.tabs,
              open: pane.tabs.open.map((tab) =>
                tab.id === result.tabId
                  ? {
                      ...tab,
                      results: result.data,
                      currentPage: 0,
                      loading: false,
                      label: newSearch,
                      searchQuery: newSearch,
                      activePassageIndex: 0,
                      footerOpen: tab.footerOpen,
                      footerTab: tab.footerTab,
                    }
                  : tab
              ),
            },
          };
        })
      );
    } catch (error) {
      console.error('Error fetching chapter:', error);
    } finally {
      setIsChapterLoading(false);
      setLoadingPaneIds(new Set());
    }
  }

  function handleSubmit(globalSearch: string, globalVersion: string, _gospel: GospelSelection) {
    const trimmedSearch = globalSearch.trim();
    if (!trimmedSearch) return;

    panes.forEach((pane) => {
      const activeTab = getActiveTab(pane);
      if (activeTab) {
        fetchPassageForTab(pane.id, activeTab.id, trimmedSearch, globalVersion, {
          includeFootnotes: true,
          includeCrossrefs: true,
          label: trimmedSearch,
        });
      }
    });
  }

  function addPane() {
    const regularPanes = panes.filter((p) => p.id !== 999);
    if (regularPanes.length >= 3) return;

    const newId = nextPaneId;
    setNextPaneId((prev) => prev + 1);

    const firstPane = panes[0];
    const firstTab = getActiveTab(firstPane);
    const versionToUse = firstPane?.version ?? translation;

    const initialTab: PaneTab = {
      id: 1,
      label: firstTab?.label ?? 'Start',
      searchQuery: firstTab?.searchQuery ?? '',
      lifecycle: 'open',
      results: [],
      currentPage: 0,
      loading: false,
      activePassageIndex: 0,
      footerOpen: false,
      footerTab: 'footnotes',
    };

    const newPane: Pane = {
      id: newId,
      version: versionToUse,
      activeTabId: 1,
      tabs: {
        open: [initialTab],
        available: [],
        closed: [],
      },
    };

    setPanes((prev) => [...prev, newPane]);

    if (firstTab && firstTab.searchQuery.trim()) {
      setTimeout(() => {
        fetchPassageForTab(newId, 1, firstTab.searchQuery, versionToUse, {
          includeFootnotes: firstTab.footerOpen && firstTab.footerTab === 'footnotes',
          includeCrossrefs: firstTab.footerOpen && firstTab.footerTab === 'crossrefs',
          label: firstTab.label,
        });
      }, 0);
    }
  }

  function toggleLanguagePane(languageCode: 'he' | 'gr') {
    if (languagePaneLanguage === languageCode) {
      setPanes((prev) => prev.filter((p) => p.id !== 999));
      setLanguagePaneLanguage(null);
      return;
    }

    setLanguagePaneLanguage(languageCode);

    const versionToUse = languageCode === 'he' ? 'WLC' : 'SBLGNT';
    const firstPane = panes[0];
    const firstTab = getActiveTab(firstPane);

    const existingLanguagePane = panes.find((p) => p.id === 999);

    if (existingLanguagePane) {
      setPanes((prev) =>
        prev.map((p) =>
          p.id === 999
            ? {
                ...p,
                version: versionToUse,
                tabs: {
                  ...p.tabs,
                  open: p.tabs.open.map((tab) =>
                    tab.id === 1
                      ? {
                          ...tab,
                          label: firstTab?.label ?? 'Start',
                          searchQuery: firstTab?.searchQuery ?? '',
                          loading: true,
                        }
                      : tab
                  ),
                },
              }
            : p
        )
      );

      if (firstTab && firstTab.searchQuery.trim()) {
        setTimeout(async () => {
          try {
            const data = await getPassage(firstTab.searchQuery, versionToUse, {
              includeFootnotes: false,
              includeCrossrefs: false,
              gospel,
            });

            const fallbackLabel = firstTab.label ?? firstTab.searchQuery;

            setPanes((prev) =>
              prev.map((p) =>
                p.id === 999
                  ? {
                      ...p,
                      tabs: {
                        ...p.tabs,
                        open: p.tabs.open.map((openTab) =>
                          openTab.id === 1
                            ? {
                                ...openTab,
                                results: data,
                                currentPage: 0,
                                loading: false,
                                label: fallbackLabel,
                                activePassageIndex: 0,
                              }
                            : openTab
                        ),
                      },
                    }
                  : p
              )
            );
          } catch (error) {
            console.error('Error fetching passage for language pane:', error);
            setPanes((prev) =>
              prev.map((p) =>
                p.id === 999
                  ? {
                      ...p,
                      tabs: {
                        ...p.tabs,
                        open: p.tabs.open.map((openTab) =>
                          openTab.id === 1 ? { ...openTab, loading: false } : openTab
                        ),
                      },
                    }
                  : p
              )
            );
          }
        }, 0);
      }
      return;
    }

    const initialTab: PaneTab = {
      id: 1,
      label: firstTab?.label ?? 'Start',
      searchQuery: firstTab?.searchQuery ?? '',
      lifecycle: 'open',
      results: [],
      currentPage: 0,
      loading: false,
      activePassageIndex: 0,
      footerOpen: false,
      footerTab: 'footnotes',
    };

    const languagePane: Pane = {
      id: 999,
      version: versionToUse,
      activeTabId: 1,
      tabs: {
        open: [initialTab],
        available: [],
        closed: [],
      },
    };

    setPanes((prev) => [...prev, languagePane]);

    if (firstTab && firstTab.searchQuery.trim()) {
      setTimeout(async () => {
        try {
          const data = await getPassage(firstTab.searchQuery, versionToUse, {
            includeFootnotes: false,
            includeCrossrefs: false,
            gospel,
          });

          const fallbackLabel = firstTab.label ?? firstTab.searchQuery;

          setPanes((prev) =>
            prev.map((p) =>
              p.id === 999
                ? {
                    ...p,
                    tabs: {
                      ...p.tabs,
                      open: p.tabs.open.map((openTab) =>
                        openTab.id === 1
                          ? {
                              ...openTab,
                              results: data,
                              currentPage: 0,
                              loading: false,
                              label: fallbackLabel,
                              activePassageIndex: 0,
                            }
                          : openTab
                      ),
                    },
                  }
                : p
            )
          );
        } catch (error) {
          console.error('Error fetching passage for language pane:', error);
          setPanes((prev) =>
            prev.map((p) =>
              p.id === 999
                ? {
                    ...p,
                    tabs: {
                      ...p.tabs,
                      open: p.tabs.open.map((openTab) =>
                        openTab.id === 1 ? { ...openTab, loading: false } : openTab
                      ),
                    },
                  }
                : p
            )
          );
        }
      }, 0);
    }
  }

  function removePane(paneId: number) {
    setPanes((prev) => prev.filter((p) => p.id !== paneId));
    setActiveIndexes((prev) => {
      const newIndexes = { ...prev };
      Object.keys(newIndexes).forEach((key) => {
        if (key.startsWith(`${paneId}-`)) {
          delete newIndexes[key];
        }
      });
      return newIndexes;
    });
  }

  function resetPanes() {
    const initialTab: PaneTab = {
      id: 1,
      label: 'Start',
      searchQuery: '',
      lifecycle: 'open',
      results: [],
      currentPage: 0,
      loading: false,
      activePassageIndex: 0,
      footerOpen: false,
      footerTab: 'footnotes',
    };

    const firstPane: Pane = {
      id: 1,
      version: 'NIV' as TranslationCode,
      activeTabId: 1,
      tabs: {
        open: [initialTab],
        available: [],
        closed: [],
      },
    };

    setPanes([firstPane]);
    setActiveIndexes({});
    setLanguagePaneLanguage(null);
  }

  function setActiveTab(paneId: number, tabId: number) {
    setPanes((prev) =>
      prev.map((pane) =>
        pane.id === paneId && pane.tabs.open.some((tab) => tab.id === tabId)
          ? { ...pane, activeTabId: tabId }
          : pane
      )
    );
  }

  function closeTab(paneId: number, tabId: number) {
    setPanes((prev) =>
      prev.map((pane) => {
        if (pane.id !== paneId) return pane;
        if (pane.tabs.open.length === 1) return pane;

        const tabToClose = pane.tabs.open.find((tab) => tab.id === tabId);
        if (!tabToClose) return pane;

        const remainingOpen = pane.tabs.open.filter((tab) => tab.id !== tabId);
        const fallbackActiveId =
          pane.activeTabId === tabId
            ? (remainingOpen[remainingOpen.length - 1]?.id ?? remainingOpen[0]?.id)
            : pane.activeTabId;

        return {
          ...pane,
          activeTabId: fallbackActiveId ?? pane.activeTabId,
          tabs: {
            ...pane.tabs,
            open: remainingOpen,
            closed: [...pane.tabs.closed, { ...tabToClose, lifecycle: 'closed' as const }],
          },
        };
      })
    );
  }

  function addTab(paneId: number, searchQuery: string, label: string): number {
    const targetPane = panes.find((p) => p.id === paneId);
    if (!targetPane) return -1;

    const newTabId = getNextTabId(targetPane);

    setPanes((prev) =>
      prev.map((pane) => {
        if (pane.id !== paneId) return pane;

        const newTab: PaneTab = {
          id: newTabId,
          label: label,
          searchQuery: searchQuery,
          lifecycle: 'open',
          results: [],
          currentPage: 0,
          loading: true,
          activePassageIndex: 0,
          footerOpen: false,
          footerTab: 'footnotes',
        };

        return {
          ...pane,
          tabs: {
            ...pane.tabs,
            open: [...pane.tabs.open, newTab],
          },
          activeTabId: newTabId,
        };
      })
    );

    return newTabId;
  }

  function handleReferenceClick(paneId: number, reference: string) {
    const newTabId = addTab(paneId, reference, reference);
    const pane = panes.find((p) => p.id === paneId);
    if (pane) {
      setTimeout(() => {
        const updatedPane = getPane(paneId);
        if (!updatedPane) return;

        fetchPassageForTab(paneId, newTabId, reference, updatedPane.version, {
          includeFootnotes: false,
          includeCrossrefs: false,
          label: reference,
        });
      }, 0);
    }
  }

  function toggleFooter(paneId: number, tabId: number, tab: 'footnotes' | 'crossrefs') {
    setPanes((prev) =>
      prev.map((p) =>
        p.id === paneId
          ? {
              ...p,
              tabs: {
                ...p.tabs,
                open: p.tabs.open.map((t) =>
                  t.id === tabId
                    ? {
                        ...t,
                        footerOpen: !t.footerOpen || t.footerTab !== tab,
                        footerTab: tab,
                      }
                    : t
                ),
              },
            }
          : p
      )
    );
  }

  function closeFooter(paneId: number, tabId: number) {
    setPanes((prev) =>
      prev.map((p) =>
        p.id === paneId
          ? {
              ...p,
              tabs: {
                ...p.tabs,
                open: p.tabs.open.map((t) => (t.id === tabId ? { ...t, footerOpen: false } : t)),
              },
            }
          : p
      )
    );
  }

  function changePaneTranslation(paneId: number, newVersion: TranslationCode) {
    setPanes((prev) => prev.map((p) => (p.id === paneId ? { ...p, version: newVersion } : p)));

    const targetPane = panes.find((p) => p.id === paneId);
    const activeTab = getActiveTab(targetPane);
    if (targetPane && activeTab && activeTab.searchQuery.trim()) {
      setTimeout(() => {
        fetchPassageForTab(
          paneId,
          activeTab.id,
          activeTab.searchQuery,
          newVersion,
          activeTab.footerOpen
            ? {
                includeFootnotes: activeTab.footerTab === 'footnotes',
                includeCrossrefs: activeTab.footerTab === 'crossrefs',
                label: activeTab.label,
              }
            : { label: activeTab.label }
        );
      }, 0);
    }
  }

  return (
    <PaneContext.Provider
      value={{
        panes,
        setPanes,
        resetPanes,
        activeIndexes,
        updateActiveIndex,
        addPane,
        toggleLanguagePane,
        languagePaneLanguage,
        removePane,
        setActiveTab,
        closeTab,
        addTab,
        toggleFooter,
        closeFooter,
        changePaneTranslation,
        fetchPassageForTab,
        fetchChapter,
        handleSubmit,
        handleReferenceClick,
        getActiveTab,
        getPane,
        shouldShowPagination,
        loadingPaneIds,
        isChapterLoading,
      }}
    >
      {children}
    </PaneContext.Provider>
  );
};

export const usePanes = () => {
  const context = useContext(PaneContext);
  if (!context) {
    throw new Error('usePanes must be used within a PaneProvider');
  }
  return context;
};
