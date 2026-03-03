import { ShellBootstrap } from './application/shell-bootstrap'

interface ShellFactoryOptions {
  preloadPath: string
  rendererHtmlPath: string
  linuxIcon?: string
  devRendererUrl?: string
}

let shellBootstrap: ShellBootstrap | undefined

export function getShellBootstrap(options: ShellFactoryOptions): ShellBootstrap {
  if (shellBootstrap === undefined) {
    shellBootstrap = new ShellBootstrap(options)
  }
  return shellBootstrap
}
