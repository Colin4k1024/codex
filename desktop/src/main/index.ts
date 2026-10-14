import { app, BrowserWindow, ipcMain } from 'electron'
import path from 'node:path'
import { normalizePlatform } from '../shared/platform'
import type { DesktopInfo } from '../shared/types'

function createWindow(): void {
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
