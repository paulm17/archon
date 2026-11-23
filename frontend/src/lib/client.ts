
import { createVardaClient } from 'varda-client';

// Shared Varda Client instance
// This ensures we only create one client and connection
// and consistently use the correct database.
export const client = createVardaClient({
  url: 'http://localhost:8000/graphql', // Adjust URL as needed or use env var
  database: 'archondb' 
});
