import { useNavigate, useParams } from 'react-router-dom';
import { NoteViewContent } from './noteViewContent';

function closeWindowOrGoBack(navigate: ReturnType<typeof useNavigate>) {
  if (window.opener) {
    window.close();
    return;
  }
  navigate('/notes/list');
}

export function NoteViewPage() {
  const navigate = useNavigate();
  const { noteId } = useParams();

  if (!noteId) {
    return null;
  }

  return <NoteViewContent noteId={noteId} onClose={() => closeWindowOrGoBack(navigate)} />;
}
