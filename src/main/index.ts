import { join } from 'node:path'
import icon from '../../resources/icon.png?asset'
import { getShellBootstrap } from './shell'

const shellBootstrap = getShellBootstrap({
  preloadPath: join(__dirname, '../preload/index.js'),
  rendererHtmlPath: join(__dirname, '../renderer/index.html'),
  linuxIcon: icon,
  devRendererUrl: process.env['ELECTRON_RENDERER_URL']
})

shellBootstrap.start()
