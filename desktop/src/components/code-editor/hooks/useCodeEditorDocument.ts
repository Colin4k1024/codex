import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../../../utils/api';
import type { CodeEditorFile } from '../types/types';
import { isBinaryFile } from '../utils/binaryFile';

type UseCodeEditorDocumentParams = {
  file: CodeEditorFile;
  projectPath?: string;
};

const getErrorMessage = (error: unknown) => {
  if (error instanceof Error) {
    return error.message;
  }

  return String(error);
};

export const useCodeEditorDocument = ({ file, projectPath }: UseCodeEditorDocumentParams) => {
  const [content, setContent] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  // U04: a save was rejected because the file changed on disk (e.g. by the agent).
  const [saveConflict, setSaveConflict] = useState(false);
  const [isBinary, setIsBinary] = useState(false);
  // Optimistic-concurrency token: the mtime we loaded, sent back on save so the
  // backend can detect concurrent modification instead of silently overwriting.
  const loadedMtimeRef = useRef<number | null>(null);
  const fileProjectName = file.projectName ?? projectPath;
  const filePath = file.path;
  const fileName = file.name;
  const fileDiffNewString = file.diffInfo?.new_string;
  const fileDiffOldString = file.diffInfo?.old_string;

  const loadDocument = useCallback(async () => {
    try {
      setLoading(true);
      setIsBinary(false);
      setSaveConflict(false);
      loadedMtimeRef.current = null;

      // Check if file is binary by extension
      if (isBinaryFile(file.name)) {
        setIsBinary(true);
        setLoading(false);
        return;
      }

      // Diff payload may already include full old/new snapshots, so avoid disk read.
      if (file.diffInfo && fileDiffNewString !== undefined && fileDiffOldString !== undefined) {
        setContent(fileDiffNewString);
        setLoading(false);
        return;
      }

      if (!fileProjectName) {
        throw new Error('Missing project identifier');
      }

      const response = await api.readFile(fileProjectName, filePath);
      if (!response.ok) {
        throw new Error(`Failed to load file: ${response.status} ${response.statusText}`);
      }

      const data = await response.json();
      if (data.error) {
        throw new Error(data.error);
      }
      setContent(data.content);
      loadedMtimeRef.current = typeof data.mtimeMs === 'number' ? data.mtimeMs : null;
    } catch (error) {
      const message = getErrorMessage(error);
      console.error('Error loading file:', error);
      setContent(`// Error loading file: ${message}\n// File: ${fileName}\n// Path: ${filePath}`);
    } finally {
      setLoading(false);
    }
  }, [file.diffInfo, file.name, fileDiffNewString, fileDiffOldString, fileName, filePath, fileProjectName]);

  useEffect(() => {
    void loadDocument();
  }, [loadDocument]);

  const handleSave = useCallback(async () => {
    setSaving(true);
    setSaveError(null);
    setSaveConflict(false);

    try {
      if (!fileProjectName) {
        throw new Error('Missing project identifier');
      }

      const response = await api.saveFile(fileProjectName, filePath, content, loadedMtimeRef.current);

      if (!response.ok) {
        const contentType = response.headers.get('content-type');
        if (contentType?.includes('application/json')) {
          const errorData = await response.json();
          throw new Error(errorData.error || `Save failed: ${response.status}`);
        }

        const textError = await response.text();
        console.error('Non-JSON error response:', textError);
        throw new Error(`Save failed: ${response.status} ${response.statusText}`);
      }

      // The desktop bridge conveys errors in-band (HTTP 200), so inspect the body
      // even on success. A conflict means the file changed on disk — do not overwrite.
      const data = await response.json();
      if (data.conflict) {
        setSaveConflict(true);
        setSaveError(data.error || 'File changed on disk. Reload to avoid overwriting newer changes.');
        return;
      }
      if (data.error) {
        throw new Error(data.error);
      }
      if (typeof data.mtimeMs === 'number') {
        loadedMtimeRef.current = data.mtimeMs;
      }

      setSaveSuccess(true);
      setTimeout(() => setSaveSuccess(false), 2000);
    } catch (error) {
      const message = getErrorMessage(error);
      console.error('Error saving file:', error);
      setSaveError(message);
    } finally {
      setSaving(false);
    }
  }, [content, filePath, fileProjectName]);

  const handleDownload = useCallback(() => {
    const blob = new Blob([content], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');

    anchor.href = url;
    anchor.download = file.name;

    document.body.appendChild(anchor);
    anchor.click();
    document.body.removeChild(anchor);

    URL.revokeObjectURL(url);
  }, [content, file.name]);

  return {
    content,
    setContent,
    loading,
    saving,
    saveSuccess,
    saveError,
    saveConflict,
    isBinary,
    handleSave,
    handleDownload,
    reload: loadDocument,
  };
};