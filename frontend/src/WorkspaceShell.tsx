import { AppShell, Tooltip } from '@mantine/core';
import { Link, Outlet, useLocation, useMatch } from 'react-router-dom';
import './WorkspaceShell.css';

const destinations = [
  { name: 'Bible', path: '/bible', icon: 'M12 6Q8 3 2 4v15q6-1 10 2 4-3 10-2V4q-6-1-10 2v15 M7 7v6m-2-4h4' },
  { name: 'Notes', path: '/notes', icon: 'M5 3h14v18H5z M3 7h4M3 12h4M3 17h4 M10 8h6m-6 4h6m-6 4h4' },
  { name: 'Ask AI', path: '/ask-ai', icon: 'M13 5H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h2v4l5-4h6a2 2 0 0 0 2-2v-3 M18 2l1.4 4.1L23 8l-3.6 1.6L18 14l-1.5-4.4L13 8l3.5-1.9z M7 12h4' },
  { name: 'Translations', path: '/translation', icon: 'M3 3h12v12H3z M9 17v4h12V9h-4 M6 12l3-7 3 7m-5-2h4 M13 18l4-4m-3 0 4 4' },
  { name: 'YT Videos', path: '/yt-video', icon: 'M5 5h14q3 0 3 3v8q0 3-3 3H5q-3 0-3-3V8q0-3 3-3z M10 8.5v7l6-3.5z' },
  { name: 'New Gospel', path: '/new_gospel', icon: 'M6 3h13v18H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z M4 18q0-2 2-2h13 M9 3v8l2-2 2 2V3' },
  { name: 'Thomas', path: '/thomas', icon: 'M6 3h12a3 3 0 0 1 3 3v2h-6V6a3 3 0 0 1 3-3 M6 3a3 3 0 0 0-3 3v2h3v10a3 3 0 0 0 3 3h9a3 3 0 0 1-3-3V8 M18 21a3 3 0 0 0 3-3v-1h-6 M8.5 10H12m-3.5 3H12' },
  { name: 'Chat', path: '/chat', icon: 'M21 11a8 8 0 0 1-8 8H9l-6 3 1.5-6A8 8 0 0 1 11 3h2a8 8 0 0 1 8 8z M8 10h8 M8 13h5' },
];

export function WorkspaceShell() {
  const { pathname } = useLocation();
  const section = '/' + pathname.split('/')[1];
  const activePath = section === '/dictionary' ? '/thomas'
    : section === '/settings' ? '/chat' : section;
  const isBible = useMatch("/bible");
  return (
    <AppShell padding={isBible ? 0 : "md"} navbar={{ width: { base: 56, sm: 72 }, breakpoint: 0 }}>
      <AppShell.Navbar className="workspace-rail" aria-label="Main navigation">
        {destinations.map(({ name, path, icon }) => (
          <Tooltip key={path} label={name} position="right" events={{ hover: true, focus: true, touch: false }}>
            <Link to={path} aria-current={activePath === path ? "page" : undefined} aria-label={name}>
              <svg aria-hidden="true" width="25" height="25" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" strokeLinecap="round">
                <path d={icon} />
              </svg>
            </Link>
          </Tooltip>
        ))}
      </AppShell.Navbar>
      <AppShell.Main><Outlet /></AppShell.Main>
    </AppShell>
  );
}
