type CreateWindowArgs = {
  title: string;
  url: string;
  width: number;
  height: number;
  x?: number;
  y?: number;
  persist_key: string;
};

declare global {
  interface Window {
    cefQuery?: (request: {
      request: string;
      onSuccess: (response: string) => void;
      onFailure: (error_code: number, error_message: string) => void;
      persistent?: boolean;
    }) => number;
  }
}

function callRust<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  return new Promise((resolve, reject) => {
    if (!window.cefQuery) {
      reject(new Error('CEF IPC unavailable'));
      return;
    }

    const requestId = crypto.randomUUID();

    window.cefQuery({
      request: JSON.stringify({ cmd, args, id: requestId }),
      persistent: false,
      onSuccess: (response) => {
        try {
          const parsed = JSON.parse(response) as {
            id: string;
            success: boolean;
            data?: T;
            error?: string;
          };
          if (!parsed.success) {
            reject(new Error(parsed.error ?? 'IPC request failed'));
            return;
          }
          resolve((parsed.data ?? parsed) as T);
        } catch {
          resolve(response as T);
        }
      },
      onFailure: (_code, message) => reject(new Error(message)),
    });
  });
}

export async function openChildWindow(args: CreateWindowArgs): Promise<void> {
  if (!window.cefQuery) {
    window.open(args.url, '_blank');
    return;
  }

  await callRust<{ status: string }>('create_window', args);
}
