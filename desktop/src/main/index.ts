import { app, BrowserWindow, ipcMain } from 'electron'
import path from 'node:path'
import { normalizePlatform } from '../shared/platform'
import type { DesktopInfo } from '../shared/types'
import { registerAppServerIpc } from './appServer/ipc'
import { DisabledAppServerManager } from './appServer/disabledManager'
import { AppServerProcessManager } from './appServer/processManager'

function isAppServerEnabled(): boolean {
  const value = process.env.CODEX_DESKTOP_APP_SERVER
  return value === '1' || value === 'true'
}

function createAppServerManager(): AppServerProcessManager | DisabledAppServerManager {
  if (!isAppServerEnabled()) {
    return new DisabledAppServerManager()
  }

  return new AppServerProcessManager({
    command: process.env.CODEX_APP_SERVER_COMMAND,
    clientInfo: {
      name: 'codex-desktop',
      title: 'Codex Desktop',
      version: app.getVersion(),
    },
  })
}

function createWindow(): BrowserWindow {
  const window = new BrowserWindow({
    width: 1280,
    height: 800,
    minWidth: 1024,
    minHeight: 640,
    show: false,
    webPreferences: {
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
    },
  })

  window.once('ready-to-show', () => {
    window.show()
    if (process.argv.includes('--smoke')) {
      app.quit()
    }
  })

  const devServerUrl = process.env.VITE_DEV_SERVER_URL
  if (devServerUrl) {
    void window.loadURL(devServerUrl)
  } else {
    void window.loadFile(path.join(__dirname, '../../dist/index.html'))
  }

  return window
}

function registerDesktopInfoHandler(): void {
  ipcMain.handle('desktop:info', (): DesktopInfo => {
    return {
      platform: normalizePlatform(process.platform),
      electronVersion: process.versions.electron,
      nodeVersion: process.versions.node,
      chromeVersion: process.versions.chrome,
    }
  })
}

app.whenReady().then(() => {
  registerDesktopInfoHandler()

  const appServerManager = createAppServerManager()
  registerAppServerIpc({
    manager: appServerManager,
    getWebContents: () => BrowserWindow.getAllWindows().map((window) => window.webContents),
  })

  app.on('before-quit', () => {
    appServerManager.dispose()
  })
  app.on('will-quit', () => {
    appServerManager.dispose()
  })

  if (isAppServerEnabled()) {
    void appServerManager.start()
  }

  createWindow()

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow()
    }
  })
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
  }
})
