
import { createVardaClient } from 'varda-client';

export const graphqlUrl = 'http://localhost:8000/graphql';

type GraphqlResponse<T> = {
  data?: T;
  errors?: Array<{
    message?: string;
  }>;
};

export async function executeVardaRequest<T>(
  database: string,
  query: string,
  variables: Record<string, unknown> = {},
): Promise<T> {
  const response = await fetch(graphqlUrl, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'x-varda-db': database,
    },
    body: JSON.stringify({
      query,
      variables,
    }),
  });

  if (!response.ok) {
    throw new Error(`GraphQL request failed with status ${response.status}`);
  }

  const json = (await response.json()) as GraphqlResponse<T>;
  if (json.errors && json.errors.length > 0) {
    throw new Error(json.errors[0]?.message || 'GraphQL request failed');
  }

  if (!json.data) {
    throw new Error('GraphQL response did not include data');
  }

  return json.data;
}

// Scripture reads come from the Archon database.
export const scriptureClient = createVardaClient({
  url: graphqlUrl,
  database: 'archondb',
});

// Reserved for this app's own gospel-specific data.
export const gospelClient = createVardaClient({
  url: graphqlUrl,
  database: 'new_gospel',
});
