// ============================================================================
// PASSAGE TYPES (from getPassage.ts)
// ============================================================================

export type BookInfo = {
  code: string;
  name: string;
};

export type PassageRef = {
  bookCode: string;
  chapter: number;
  verseStart: number;
  verseEnd: number;
};

export type FootnoteItem = {
  marker?: string;
  chapter: number;
  verse: number;
  noteId: string;
  text: string;
  references?: any;
};

export type CrossrefItem = {
  chapter: number;
  verse: number;
  refId: string;
  links?: any;
};

export type PassageResult = {
  reference: PassageRef;
  html: string;
  referenceText: string;
  isWholeChapter?: boolean;
  maxChapter?: number;
  footnotes?: FootnoteItem[];
  crossrefs?: CrossrefItem[];
};

// ============================================================================
// BOOK & CHAPTER TYPES (from getBooksGrouped.ts and getBookChapters.ts)
// ============================================================================

export type ChapterHeading = {
  number: number;
  heading: string;
};

export type BookWithHeadings = {
  id: string;
  code: string;
  name: string;
  chapters: ChapterHeading[];
};

export type CategoryGroup = {
  name: string; // "Old Testament", "New Testament", "Apocrypha"
  books: BookWithHeadings[];
};

export type BooksGroup = {
  translation: string;
  language: string | null;
  categories: CategoryGroup[];
};

// ============================================================================
// TRANSLATION TYPES (from getTranslations.ts)
// ============================================================================

export type TranslationCode = string;

export type Translation = {
  code: string;
  name: string;
};

export type GospelSelection = 'canonical' | 'gospel-of-thomas';

// ============================================================================
// UI / PANE TYPES (from pane.tsx, passageSection.tsx)
// ============================================================================

export type TabLifecycle = 'open' | 'available' | 'closed';

export interface PaneTab {
  id: number;
  label: string;
  searchQuery: string;
  lifecycle: TabLifecycle;
  results: PassageResult[];
  currentPage: number;
  loading: boolean;
  activePassageIndex: number;
  // MOVED FROM PANE TO TAB
  footerOpen: boolean;
  footerTab: 'footnotes' | 'crossrefs';
}

export interface PaneTabsBuckets {
  open: PaneTab[];
  available: PaneTab[];
  closed: PaneTab[];
}

export interface Pane {
  id: number;
  version: string;
  tabs: PaneTabsBuckets;
  activeTabId: number;
  // REMOVED footerOpen and footerTab - now on each tab
}

// ============================================================================
// OBSERVER TYPES (from pane.tsx)
// ============================================================================

export type PaneObserverState = {
  root: HTMLElement | null;
  observer: IntersectionObserver;
  entries: Map<number, { el: HTMLElement; ratio: number }>; // sectionIndex -> data
  rafId: number | null;
  debounceTimer: number | null;
  lastTopIndex: number | null;
};
