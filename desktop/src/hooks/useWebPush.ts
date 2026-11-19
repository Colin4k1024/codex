import { useCallback, useState } from 'react';

// Product modification (AGPL Section 7 — see desktop/SOURCE.md). U06 removed Web Push:
// the desktop product has no service worker / pushManager / VAPID subscription and no
// localhost push endpoints. The hook keeps its shape so the settings UI compiles, but
// reports 'unsupported' and no-ops; the Web Push section is hidden in desktop mode.
type WebPushState = {
  permission: NotificationPermission | 'unsupported';
  isSubscribed: boolean;
  isLoading: boolean;
  subscribe: () => Promise<void>;
  unsubscribe: () => Promise<void>;
};

export function useWebPush(): WebPushState {
  const [isSubscribed] = useState(false);
  const [isLoading] = useState(false);
  const subscribe = useCallback(async () => {}, []);
  const unsubscribe = useCallback(async () => {}, []);
  return { permission: 'unsupported', isSubscribed, isLoading, subscribe, unsubscribe };
}