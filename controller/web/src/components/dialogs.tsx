import { useState, useCallback, useEffect, useRef } from 'react'

export interface ConfirmOptions {
  title: string
  message: string
  confirmLabel?: string
  danger?: boolean
}

interface ConfirmState extends ConfirmOptions {
  resolve: (ok: boolean) => void
}

export function useConfirm() {
  const [state, setState] = useState<ConfirmState | null>(null)

  const confirmDialog = useCallback((opts: ConfirmOptions) => {
    return new Promise<boolean>(resolve => setState({ ...opts, resolve }))
  }, [])

  const close = (ok: boolean) => {
    state?.resolve(ok)
    setState(null)
  }

  const confirmElement = state ? (
    <div className="fixed inset-0 z-[70] flex items-center justify-center px-4 bg-black/60 backdrop-blur-sm" onClick={() => close(false)}>
      <div className="w-full max-w-md bg-[#15131f] border border-white/10 rounded-xl shadow-2xl p-6 flex flex-col gap-4" onClick={e => e.stopPropagation()}>
        <h3 className="text-lg font-semibold text-zinc-100 m-0">{state.title}</h3>
        <p className="text-sm text-zinc-400 m-0 whitespace-pre-wrap">{state.message}</p>
        <div className="flex items-center justify-end gap-3 mt-2">
          <button
            className="px-4 py-2 text-sm font-medium text-zinc-400 hover:text-zinc-200 transition-colors"
            onClick={() => close(false)}
          >
            Cancel
          </button>
          <button
            autoFocus
            className={`px-5 py-2 text-sm font-medium text-white rounded-md border shadow-sm transition-all ${
              state.danger
                ? 'bg-red-600 hover:bg-red-500 border-red-500/50'
                : 'bg-purple-600 hover:bg-purple-500 border-purple-500/50'
            }`}
            onClick={() => close(true)}
          >
            {state.confirmLabel ?? 'Confirm'}
          </button>
        </div>
      </div>
    </div>
  ) : null

  return { confirmDialog, confirmElement }
}

export interface ToastState {
  kind: 'success' | 'error'
  message: string
}

export function useToast() {
  const [toast, setToast] = useState<ToastState | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const showToast = useCallback((kind: ToastState['kind'], message: string) => {
    if (timer.current) clearTimeout(timer.current)
    setToast({ kind, message })
    timer.current = setTimeout(() => setToast(null), 5000)
  }, [])

  useEffect(() => () => { if (timer.current) clearTimeout(timer.current) }, [])

  const toastElement = toast ? (
    <div className="fixed bottom-6 right-6 z-[80] max-w-md">
      <div
        className={`flex items-start gap-3 px-4 py-3 rounded-lg border shadow-2xl text-sm ${
          toast.kind === 'success'
            ? 'bg-[#0f1a12] border-green-500/30 text-green-300'
            : 'bg-[#1a0f12] border-red-500/30 text-red-300'
        }`}
      >
        <span className="break-words whitespace-pre-wrap">{toast.message}</span>
        <button className="text-zinc-500 hover:text-zinc-300 transition-colors shrink-0" onClick={() => setToast(null)}>✕</button>
      </div>
    </div>
  ) : null

  return { showToast, toastElement }
}
