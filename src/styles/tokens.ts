export interface RendererDesignTokens {
  brand: {
    primary: string
    primaryStrong: string
    accent: string
    onPrimary: string
  }
  color: {
    bg: string
    bgElevated: string
    surface: {
      0: string
      1: string
      2: string
      3: string
    }
    text: {
      primary: string
      secondary: string
      tertiary: string
      disabled: string
      inverse: string
      onMedia: string
    }
    border: {
      subtle: string
      default: string
      strong: string
      divider: string
    }
    state: {
      hover: string
      pressed: string
      selected: string
      disabled: string
    }
    status: {
      success: string
      warning: string
      danger: string
      info: string
    }
    focus: {
      ring: string
      ringOuter: string
    }
  }
  motion: {
    ease: {
      standard: string
      emphasized: string
      linear: string
    }
    duration: Record<'80' | '120' | '160' | '200' | '240' | '320', string>
    scale: {
      hover: number
      focus: number
      pressed: number
    }
  }
  layout: {
    containerMax: string
    navHeight: string
    safePaddingX: string
    breakpoints: {
      sm: number
      md: number
      lg: number
      xl: number
    }
  }
  component: {
    tile: {
      radius: string
      aspect: {
        game: string
        console: string
      }
    }
    settings: {
      navWidth: string
      itemHeight: string
      itemRadius: string
    }
    drawer: {
      width: string
      widthLg: string
    }
  }
}

// 先提供稳定常量，后续可接入构建脚本自动生成。
export const rendererTokens: RendererDesignTokens = {
  brand: {
    primary: '#107C10',
    primaryStrong: '#16a316',
    accent: '#2BC24A',
    onPrimary: '#ffffff',
  },
  color: {
    bg: '#0f0f10',
    bgElevated: '#121214',
    surface: {
      0: '#0f0f10',
      1: '#151518',
      2: '#19191d',
      3: '#1f1f24',
    },
    text: {
      primary: '#ffffff',
      secondary: '#cfcfd6',
      tertiary: '#9b9ba6',
      disabled: '#6f6f7a',
      inverse: '#0a0a0a',
      onMedia: '#ffffff',
    },
    border: {
      subtle: 'rgba(255, 255, 255, 0.08)',
      default: 'rgba(255, 255, 255, 0.14)',
      strong: 'rgba(255, 255, 255, 0.22)',
      divider: 'rgba(255, 255, 255, 0.10)',
    },
    state: {
      hover: 'rgba(255, 255, 255, 0.06)',
      pressed: 'rgba(255, 255, 255, 0.10)',
      selected: 'rgba(16, 124, 16, 0.22)',
      disabled: 'rgba(255, 255, 255, 0.04)',
    },
    status: {
      success: '#2BC24A',
      warning: '#f7b500',
      danger: '#ff4d4f',
      info: '#4da3ff',
    },
    focus: {
      ring: '#7CFF7C',
      ringOuter: 'rgba(124, 255, 124, 0.40)',
    },
  },
  motion: {
    ease: {
      standard: 'cubic-bezier(0.2, 0, 0, 1)',
      emphasized: 'cubic-bezier(0.2, 0, 0, 1.2)',
      linear: 'linear',
    },
    duration: {
      80: '80ms',
      120: '120ms',
      160: '160ms',
      200: '200ms',
      240: '240ms',
      320: '320ms',
    },
    scale: {
      hover: 1.03,
      focus: 1.04,
      pressed: 0.99,
    },
  },
  layout: {
    containerMax: '1440px',
    navHeight: '56px',
    safePaddingX: '16px',
    breakpoints: {
      sm: 600,
      md: 840,
      lg: 1024,
      xl: 1280,
    },
  },
  component: {
    tile: {
      radius: '16px',
      aspect: {
        game: '16/9',
        console: '3/2',
      },
    },
    settings: {
      navWidth: '320px',
      itemHeight: '56px',
      itemRadius: '12px',
    },
    drawer: {
      width: '360px',
      widthLg: '420px',
    },
  },
}
