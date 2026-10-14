import { contextBridge, ipcRenderer } from 'electron'
import type { CodexDesktopBridge } from '../shared/types'

const bridge: CodexDesktopBridge = {
  getDesktopInfo: () => ipcRenderer.invoke('desktop:info'),
}

contextBridge.exposeInMainWorld('codexDesktop', bridge)
