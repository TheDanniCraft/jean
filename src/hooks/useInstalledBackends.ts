import { useMemo } from 'react'
import { useClaudeCliStatus } from '@/services/claude-cli'
import { useCodexCliStatus } from '@/services/codex-cli'
import { useOpencodeCliStatus } from '@/services/opencode-cli'
import { useGeminiCliStatus } from '@/services/gemini-cli'
import type { CliBackend } from '@/types/preferences'

/**
 * Returns only the backends whose CLIs are currently installed.
 * Use this to filter backend selection UI so users can't pick uninstalled ones.
 */
export function useInstalledBackends(options?: { enabled?: boolean }) {
  const enabled = options?.enabled ?? true
  const claude = useClaudeCliStatus({ enabled })
  const codex = useCodexCliStatus({ enabled })
  const opencode = useOpencodeCliStatus({ enabled })
  const gemini = useGeminiCliStatus({ enabled })

  const installedBackends = useMemo(() => {
    const backends: CliBackend[] = []
    if (claude.data?.installed) backends.push('claude')
    if (codex.data?.installed) backends.push('codex')
    if (opencode.data?.installed) backends.push('opencode')
    if (gemini.data?.installed) backends.push('gemini')
    return backends
  }, [
    claude.data?.installed,
    codex.data?.installed,
    opencode.data?.installed,
    gemini.data?.installed,
  ])

  return {
    installedBackends,
    isLoading:
      claude.isLoading ||
      codex.isLoading ||
      opencode.isLoading ||
      gemini.isLoading,
  }
}
