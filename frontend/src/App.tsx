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

import { NewGospelPage } from './newGospel/newGospelPage';
import { ThomasPage } from './thomas/thomasPage';
import { DictionaryPage } from './thomas/dictionaryPage';
import { SettingsPage } from './settings/settingsPage';
import { ChatPage } from './chat/chatPage';

function GospelLayout() {
  const location = useLocation();
  const isChildWindow = new URLSearchParams(location.search).get('child') === '1';

  if (isChildWindow) return <Outlet />;

  return (
    <Container size="100%" py="lg" style={{ height: '100vh', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <Outlet />
    </Container>
  );
}

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
          <Route element={<GospelLayout />}>
            <Route path="new_gospel" element={<NewGospelPage />} />
            <Route path="thomas" element={<ThomasPage />} />
            <Route path="dictionary/:tokenId" element={<DictionaryPage />} />
          </Route>
          <Route path="chat" element={<ChatPage />} />
          <Route path="settings" element={<SettingsPage />} />
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
