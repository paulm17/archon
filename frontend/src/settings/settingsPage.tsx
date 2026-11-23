import { Text, Container, Button, Stack, Group, Paper } from '@mantine/core';
import { useState, useEffect } from 'react';

export function SettingsPage() {
  const [loadedModel, setLoadedModel] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const checkStatus = async () => {
    try {
      const res = await fetch('http://localhost:8080/v1/models');
      if (res.ok) {
        const data = await res.json();
        if (data.data && data.data.length > 0) {
          setLoadedModel(data.data[0].id);
        } else {
          setLoadedModel(null);
        }
        setError(null);
      } else {
        setError('LLM Server returned an error');
      }
    } catch (e) {
      console.error('Failed to check LLM status', e);
      setError('LLM Server not reachable');
    }
  };

  useEffect(() => {
    checkStatus();
  }, []);

  const handleLoad = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch('http://localhost:8080/llm/load', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ model_path: 'unsloth/gemma-4-E4B-it-MLX-8bit' }),
      });
      
      if (res.ok) {
        // Wait a bit for the server to update its state if necessary
        await checkStatus();
      } else {
        const errData = await res.json().catch(() => ({}));
        setError(errData.error?.message || 'Failed to load model. Ensure MLX-RS is running.');
      }
    } catch (e) {
      setError('Failed to communicate with LLM server. Is MLX-RS running on port 8080?');
    } finally {
      setLoading(false);
    }
  };

  const handleUnload = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch('http://localhost:8080/llm/unload', { 
        method: 'POST' 
      });
      if (res.ok) {
        setLoadedModel(null);
      } else {
        setError('Failed to unload model');
      }
    } catch (e) {
      setError('Failed to communicate with LLM server');
    } finally {
      setLoading(false);
    }
  };

  const formatModelName = (name: string | null) => {
    if (!name) return name;
    if (name.includes('models--')) {
      return name.split('models--').pop()?.split('/')[0].split('--').pop() || name;
    }
    return name.split('/').pop() || name;
  };

  return (
    <Container size="100%" py="md">
      <Stack gap="xl">
        <div>
          <Text size="xl" fw={700}>Settings</Text>
          <Text size="sm" c="dimmed">Configure your application and model parameters.</Text>
        </div>
        
        <Paper withBorder p="md" radius="md">
          <Stack gap="md">
            <div>
              <Text fw={600}>LLM Model Management (MLX)</Text>
              <Text size="sm" c="dimmed">Manage the currently loaded model on your local MLX server.</Text>
            </div>

            <Group align="flex-end">
              {loadedModel ? (
                <Button 
                  color="red" 
                  onClick={handleUnload} 
                  loading={loading}
                  variant="filled"
                >
                  Unload Model
                </Button>
              ) : (
                <Button 
                  color="green" 
                  onClick={handleLoad} 
                  loading={loading}
                  variant="filled"
                >
                  Load Model
                </Button>
              )}
              
              <Stack gap={2}>
                <Text size="xs" fw={500} c="dimmed">SERVER STATUS</Text>
                <Group gap="xs">
                  <div style={{ 
                    width: 8, 
                    height: 8, 
                    borderRadius: '50%', 
                    backgroundColor: error ? 'var(--mantine-color-red-6)' : 'var(--mantine-color-green-6)' 
                  }} />
                  <Text size="sm">
                    {loadedModel ? `Loaded: ${formatModelName(loadedModel)}` : 'Standby / Ready to Load'}
                  </Text>
                </Group>
              </Stack>
            </Group>

            {error && (
              <Text size="xs" c="red" fw={500}>{error}</Text>
            )}
          </Stack>
        </Paper>
      </Stack>
    </Container>
  );
}
