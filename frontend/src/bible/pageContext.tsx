'use client';

import { createContext, useContext, useState } from 'react';
import type { ReactNode } from 'react';
import type { BooksGroup, GospelSelection, Translation, TranslationCode } from './types';

interface PageContextState {
  // Search input
  search: string;
  setSearch: (s: string) => void;

  // Translation selection
  version: TranslationCode;
  setVersion: (v: TranslationCode) => void;
  gospel: GospelSelection;
  setGospel: (g: GospelSelection) => void;

  // Available translations
  translations: Translation[];
  setTranslations: (t: Translation[]) => void;

  // Books data
  booksGrouped: BooksGroup | null;
  setBooksGrouped: (b: BooksGroup | null) => void;

  // Loading states
  loadingBooks: boolean;
  setLoadingBooks: (v: boolean) => void;

  loadingChapters: boolean;
  setLoadingChapters: (v: boolean) => void;

  // Selected book in the book list
  selectedBook: { id: string; code: string } | null;
  setSelectedBook: (v: { id: string; code: string } | null) => void;

  // First search tracking
  hasSearched: boolean;
  setHasSearched: (v: boolean) => void;

  // Helper function
  getFullBookName: (bookCode: string, booksGrouped: BooksGroup | null) => string;
}

const PageContext = createContext<PageContextState | null>(null);

export const PageProvider = ({ children }: { children: ReactNode }) => {
  const [search, setSearch] = useState('');
  const [version, setVersion] = useState<TranslationCode>('' as TranslationCode);
  const [gospel, setGospel] = useState<GospelSelection>('canonical');
  const [translations, setTranslations] = useState<Translation[]>([]);
  const [booksGrouped, setBooksGrouped] = useState<BooksGroup | null>(null);
  const [loadingBooks, setLoadingBooks] = useState(false);
  const [loadingChapters, setLoadingChapters] = useState(false);
  const [selectedBook, setSelectedBook] = useState<{ id: string; code: string } | null>(null);
  const [hasSearched, setHasSearched] = useState(false);

  // Helper to get full book name from book code
  const getFullBookName = (bookCode: string, booksGrouped: BooksGroup | null): string => {
    if (!booksGrouped) return bookCode;

    for (const category of booksGrouped.categories) {
      const book = category.books.find((b) => b.code === bookCode);
      if (book) {
        return book.name;
      }
    }
    return bookCode;
  };

  return (
    <PageContext.Provider
      value={{
        search,
        setSearch,
        version,
        setVersion,
        gospel,
        setGospel,
        translations,
        setTranslations,
        booksGrouped,
        setBooksGrouped,
        loadingBooks,
        setLoadingBooks,
        loadingChapters,
        setLoadingChapters,
        selectedBook,
        setSelectedBook,
        hasSearched,
        setHasSearched,
        getFullBookName,
      }}
    >
      {children}
    </PageContext.Provider>
  );
};

export const usePage = () => {
  const context = useContext(PageContext);
  if (!context) {
    throw new Error('usePage must be used within a PageProvider');
  }
  return context;
};
