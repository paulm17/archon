import { create } from 'zustand';
import { persist } from 'zustand/middleware';

interface ThomasState {
  currentLogionNumber: string;
  setCurrentLogionNumber: (logionNumber: string | ((prev: string) => string)) => void;
}

export const useThomasStore = create<ThomasState>()(
  persist(
    (set) => ({
      currentLogionNumber: '1',
      setCurrentLogionNumber: (logionNumber) =>
        set((state) => ({
          currentLogionNumber: typeof logionNumber === 'function' ? logionNumber(state.currentLogionNumber) : logionNumber,
        })),
    }),
    {
      name: 'thomas-storage',
    },
  ),
);
