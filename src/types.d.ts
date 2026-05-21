declare global {
  interface Window {
    TencentCaptcha?: new (
      appId: string,
      callback: (res: {
        ret: number;
        ticket?: string;
        randstr?: string;
        errorCode?: string;
        errorMessage?: string;
      }) => void,
      options?: Record<string, unknown>
    ) => { show: () => void };
  }
}

export {};
