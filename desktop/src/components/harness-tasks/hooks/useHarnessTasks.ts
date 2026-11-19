import { useCallback, useEffect, useState } from 'react';
import {
  desktopClient,
  type HarnessTaskDetail,
  type HarnessTaskSummary,
} from '../../../services/desktop-client';

// Harness business-run state is owned by Rust (orchestrator + storage). This hook
// only reads typed command results; it never infers node success from TaskMaster
// identifiers or chat text. Gate evidence (exit_code / evidence_ref / proven) is
// rendered verbatim from the backend.
export function useHarnessTasks(isVisible: boolean) {
  const [tasks, setTasks] = useState<HarnessTaskSummary[]>([]);
  const [selected, setSelected] = useState<HarnessTaskDetail | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [artifactView, setArtifactView] = useState<{ id: string; content: string } | null>(null);

  const refreshList = useCallback(async () => {
    try {
      const res = await desktopClient.harnessListTasks();
      setTasks(res.tasks ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const selectTask = useCallback(async (taskId: string) => {
    setIsLoading(true);
    setError(null);
    setArtifactView(null);
    try {
      const detail = await desktopClient.harnessGetTask(taskId);
      if (detail.error) {
        setError(detail.error);
        setSelected(null);
      } else {
        setSelected(detail);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsLoading(false);
    }
  }, []);

  const createTask = useCallback(async (title: string, mode?: string) => {
    setIsCreating(true);
    setError(null);
    try {
      const detail = await desktopClient.harnessCreateTask(title, mode);
      if (detail.error) {
        setError(detail.error);
      } else {
        setSelected(detail);
        await refreshList();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsCreating(false);
    }
  }, [refreshList]);

  const readArtifact = useCallback(async (taskId: string, artifactId: string) => {
    try {
      const res = await desktopClient.harnessReadArtifact(taskId, artifactId);
      if (res.error) {
        setError(res.error);
      } else {
        setArtifactView({ id: artifactId, content: res.content ?? '' });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    if (isVisible) void refreshList();
  }, [isVisible, refreshList]);

  return {
    tasks,
    selected,
    isLoading,
    isCreating,
    error,
    artifactView,
    clearError: () => setError(null),
    clearArtifact: () => setArtifactView(null),
    refreshList,
    selectTask,
    createTask,
    readArtifact,
  };
}