import { Navigate, Outlet } from 'react-router-dom';

export function NotesPage() {
  return <Outlet />;
}

export function NotesIndexRedirect() {
  return <Navigate to="/notes/list" replace />;
}
