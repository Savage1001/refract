const { app, BrowserWindow } = require('electron');
app.whenReady().then(() => { const window = new BrowserWindow({ show: false, webPreferences: { contextIsolation: false } }); void window.loadURL('data:text/html,<html><body></body></html>'); });
app.on('window-all-closed', () => app.quit());
