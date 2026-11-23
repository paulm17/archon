import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { VardaProvider } from 'varda-client'
import { MantineProvider } from '@mantine/core';
import { scriptureClient } from './lib/client'
import App from './App'

import './index.css'
import '@mantine/core/styles.css';

// Render the app
const rootElement = document.getElementById('root')!
if (!rootElement.innerHTML) {
  const root = createRoot(rootElement)
  root.render(
    <StrictMode>
      <VardaProvider value={scriptureClient}>
        <MantineProvider>
          <App />
        </MantineProvider>
      </VardaProvider>
    </StrictMode>,
  )
}
