import { BrowserRouter, Routes, Route, useNavigate, useLocation, Outlet, Navigate } from 'react-router-dom';
import { Container, Tabs } from '@mantine/core';
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
  const navigate = useNavigate();
  const location = useLocation();
  const currentTab = location.pathname.split('/')[1] || 'bible';
  const isChildWindow = new URLSearchParams(location.search).get('child') === '1';

  return (
    <Container size="95%" py="lg">
      {isChildWindow ? (
        <Outlet />
      ) : (
        <Tabs value={currentTab} onChange={(value) => navigate(`/${value}`)}>
          <Tabs.List>
            <Tabs.Tab value="bible">Bible</Tabs.Tab>
            <Tabs.Tab value="notes">Notes</Tabs.Tab>
            <Tabs.Tab value="ask-ai">Ask AI</Tabs.Tab>
            <Tabs.Tab value="translation">Translations</Tabs.Tab>
            <Tabs.Tab value="yt-video">YT Videos</Tabs.Tab>
          </Tabs.List>
          <div style={{ paddingTop: 16 }}>
            <Outlet />
          </div>
        </Tabs>
      )}
    </Container>
  );
}

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<Navigate to="/bible" replace />} />
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
