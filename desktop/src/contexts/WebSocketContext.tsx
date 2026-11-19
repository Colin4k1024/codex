import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { desktopClient } from '../services/desktop-client';

// Product modification (AGPL Section 7 — see desktop/SOURCE.md). U06 removed the
// upstream OSS WebSocket transport: the desktop product receives ordered product
// events over Tauri IPC (desktopClient.subscribe) and sends input via typed commands.
// There is no `new WebSocket` and no localhost /ws endpoint. The `ws` field is kept
// (always null) only for downstream prop-shape compatibility with the legacy chat UI.
type WebSocketContextType = {
  ws: WebSocket | null;
  sendMessage: (message: any) => void;
  latestMessage: any | null;
  isConnected: boolean;
};

const WebSocketContext = createContext<WebSocketContextType | null>(null);

export const useWebSocket = () => {
  const context = useContext(WebSocketContext);
  if (!context) {
    throw new Error('useWebSocket must be used within a WebSocketProvider');
  }
  return context;
};

const useWebSocketProviderState = (): WebSocketContextType => {
  const unmountedRef = useRef(false);
  const unlistenRef = useRef<(() => void) | null>(null);
  const [latestMessage, setLatestMessage] = useState<any>(null);
  const [isConnected, setIsConnected] = useState(false);

  useEffect(() => {
    unmountedRef.current = false;
    // Subscribe first so no engine event is missed, then connect the real Codex
    // app-server adapter (U03). When no engine is configured the commands fall back
    // to fixtures and the shell still boots.
    setIsConnected(true);
    void desktopClient
      .subscribe((payload) => {
        if (!unmountedRef.current) setLatestMessage(payload);
      })
      .then((unlisten) => {
        unlistenRef.current = unlisten;
        return desktopClient.engineConnect();
      })
      .then((status) => {
        if (!status.connected && status.reason) {
          console.info('Desktop engine not connected:', status.reason);
        }
      })
      .catch((error) => {
        console.error('Desktop event subscription/engine connect failed:', error);
      });

    return () => {
      unmountedRef.current = true;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    };
  }, []);

  const sendMessage = useCallback((message: any) => {
    void desktopClient.sendInput(message).catch((error) => {
      console.error('Desktop sendInput failed:', error);
    });
  }, []);

  const value: WebSocketContextType = useMemo(() => ({
    ws: null,
    sendMessage,
    latestMessage,
    isConnected,
  }), [sendMessage, latestMessage, isConnected]);

  return value;
};

export const WebSocketProvider = ({ children }: { children: React.ReactNode }) => {
  const webSocketData = useWebSocketProviderState();

  return (
    <WebSocketContext.Provider value={webSocketData}>
      {children}
    </WebSocketContext.Provider>
  );
};

export default WebSocketContext;