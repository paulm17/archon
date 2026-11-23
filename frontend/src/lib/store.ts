import { create } from 'zustand';
import { persist } from 'zustand/middleware';

interface GospelState {
  currentVerseNumber: number;
  setCurrentVerseNumber: (verseNumber: number | ((prev: number) => number)) => void;
}

export const useGospelStore = create<GospelState>()(
  persist(
    (set) => ({
      currentVerseNumber: 1,
      setCurrentVerseNumber: (verseNumber) =>
        set((state) => ({
          currentVerseNumber: typeof verseNumber === 'function' ? verseNumber(state.currentVerseNumber) : verseNumber,
        })),
    }),
    {
      name: 'gospel-storage',
    },
  ),
);

interface ChatState {
  currentThreadId: string | null;
  setCurrentThreadId: (threadId: string | null) => void;
}

export const useChatStore = create<ChatState>((set) => ({
  currentThreadId: null,
  setCurrentThreadId: (threadId) => set({ currentThreadId: threadId }),
}));