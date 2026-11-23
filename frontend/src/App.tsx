import { BrowserRouter, Routes, Route, useLocation, Outlet, Navigate } from 'react-router-dom';
import { Container } from '@mantine/core';
import { WorkspaceShell } from './WorkspaceShell';
import BiblePage from './bible/biblePage';
import { AskAiPage } from './ask-ai/askAiPage';
import { CategoriesPage } from './note-ui/categoriesPage';
import { CategoryEditorPage } from './note-ui/categoryEditorPage';
import { NoteEditorPage } from './note-ui/noteEditorPage';
import { NotesIndexRedirect, NotesPage } from './note-ui/notesPage';
import { NotesListPage } from './note-ui/notesListPage';
import { NoteViewPage } from './note-ui/noteViewPage';
import { TranslationPage } from './translation/translationPage';
import { YTVideoPage } from './ytvideo/ytVideoPage';

function Layout() {
  const { search } = useLocation();
  if (new URLSearchParams(search).get('child') === '1') {
    return <Container size="95%" py="lg"><Outlet /></Container>;
  }

  return <WorkspaceShell />;
}

function BibleRedirect() {
  const { search } = useLocation();
  return <Navigate to={{ pathname: '/bible', search }} replace />;
}

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<BibleRedirect />} />
          <Route path="bible" element={<BiblePage />} />
          <Route path="notes" element={<NotesPage />}>
            <Route index element={<NotesIndexRedirect />} />
            <Route path="list" element={<NotesListPage />} />
            <Route path="new" element={<NoteEditorPage />} />
            <Route path=":noteId/view" element={<NoteViewPage />} />
            <Route path=":noteId/edit" element={<NoteEditorPage />} />
            <Route path="categories" element={<CategoriesPage />} />
            <Route path="categories/new" element={<CategoryEditorPage />} />
            <Route path="categories/:categoryId/edit" element={<CategoryEditorPage />} />
          </Route>
          <Route path="ask-ai" element={<AskAiPage />} />
          <Route path="translations" element={<Navigate to="/translation" replace />} />
          <Route path="translation" element={<TranslationPage />} />
          {/* Changed path to match the tab value 'yt-video' */}
          <Route path="yt-video" element={<YTVideoPage />} />
          <Route path="*" element={<div>Page not found</div>} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
