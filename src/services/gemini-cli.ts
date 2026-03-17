import { useCallback, useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { invoke, listen, useWsConnectionStatus } from '@/lib/transport'
import { hasBackend } from '@/lib/environment'
import { logger } from '@/lib/logger'
import { toast } from 'sonner'

interface GeminiCliStatus {
  installed: boolean
  version: string | null
  path: string | null
}

export interface GeminiAuthStatus {
  authenticated: boolean
  error: string | null
  email?: string
  project?: string
  tier?: string
}

export interface GeminiUsageBucket {
  model_id: string
  remaining_fraction: number
  usage_percent: number
  reset_time: number | null
}

export interface GeminiUsageSnapshot {
  quotas: GeminiUsageBucket[]
  fetched_at: number
  plan_name: string | null
  tier: string | null
}

interface GeminiReleaseInfo {
  version: string
  tagName: string
  publishedAt: string
  prerelease: boolean
}

interface GeminiInstallProgress {
  stage: string
  message: string
  percent: number
}

const isTauri = hasBackend

export const geminiCliQueryKeys = {
  all: ['gemini-cli'] as const,
  status: () => [...geminiCliQueryKeys.all, 'status'] as const,
  auth: () => [...geminiCliQueryKeys.all, 'auth'] as const,
  usage: () => [...geminiCliQueryKeys.all, 'usage'] as const,
  versions: () => [...geminiCliQueryKeys.all, 'versions'] as const,
}

export function useGeminiUsage(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: geminiCliQueryKeys.usage(),
    queryFn: async (): Promise<GeminiUsageSnapshot> => {
      if (!isTauri()) {
        throw new Error('Not in Tauri context')
      }
      return await invoke<GeminiUsageSnapshot>('get_gemini_usage')
    },
    enabled: options?.enabled ?? true,
    staleTime: 1000 * 60 * 5,
    gcTime: 1000 * 60 * 10,
    refetchInterval: 1000 * 60 * 5,
  })
}

export function useGeminiCliStatus(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: geminiCliQueryKeys.status(),
    queryFn: async (): Promise<GeminiCliStatus> => {
      if (!isTauri()) {
        return { installed: false, version: null, path: null }
      }

      try {
        return await invoke<GeminiCliStatus>('check_gemini_cli_installed')
      } catch (error) {
        logger.error('Failed to check Gemini CLI status', { error })
        return { installed: false, version: null, path: null }
      }
    },
    enabled: options?.enabled ?? true,
    staleTime: 1000 * 60 * 5,
    gcTime: 1000 * 60 * 10,
    refetchInterval: 1000 * 60 * 60,
  })
}

export function useGeminiCliAuth(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: geminiCliQueryKeys.auth(),
    queryFn: async (): Promise<GeminiAuthStatus> => {
      if (!isTauri()) {
        return { authenticated: false, error: 'Not in Tauri context' }
      }

      try {
        return await invoke<GeminiAuthStatus>('check_gemini_cli_auth')
      } catch (error) {
        logger.error('Failed to check Gemini CLI auth', { error })
        return {
          authenticated: false,
          error: error instanceof Error ? error.message : String(error),
        }
      }
    },
    enabled: options?.enabled ?? true,
    staleTime: 1000 * 60 * 5,
    gcTime: 1000 * 60 * 10,
  })
}

export function useAvailableGeminiVersions(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: geminiCliQueryKeys.versions(),
    queryFn: async (): Promise<GeminiReleaseInfo[]> => {
      if (!isTauri()) return []

      try {
        const versions = await invoke<
          {
            version: string
            tag_name: string
            published_at: string
            prerelease: boolean
          }[]
        >('get_available_gemini_versions')

        return versions.map(v => ({
          version: v.version,
          tagName: v.tag_name,
          publishedAt: v.published_at,
          prerelease: v.prerelease,
        }))
      } catch (error) {
        logger.error('Failed to fetch Gemini CLI versions', { error })
        throw error
      }
    },
    enabled: options?.enabled ?? true,
    staleTime: 1000 * 60 * 15,
    gcTime: 1000 * 60 * 30,
    refetchInterval: 1000 * 60 * 60,
  })
}

export function useInstallGeminiCli() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (version?: string) => {
      await invoke('install_gemini_cli', { version: version ?? null })
    },
    retry: false,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: geminiCliQueryKeys.status() })
      toast.success('Gemini CLI installed successfully')
    },
    onError: error => {
      const message = error instanceof Error ? error.message : String(error)
      toast.error('Failed to install Gemini CLI', { description: message })
    },
  })
}

export function useGeminiInstallProgress(): [
  GeminiInstallProgress | null,
  () => void,
] {
  const [progress, setProgress] = useState<GeminiInstallProgress | null>(null)
  const wsConnected = useWsConnectionStatus()

  const resetProgress = useCallback(() => {
    setProgress(null)
  }, [])

  useEffect(() => {
    if (!isTauri()) return
    let unlistenFn: (() => void) | null = null

    const setupListener = async () => {
      unlistenFn = await listen<GeminiInstallProgress>(
        'gemini-cli:install-progress',
        event => setProgress(event.payload)
      )
    }

    setupListener()
    return () => {
      if (unlistenFn) unlistenFn()
    }
  }, [wsConnected])

  return [progress, resetProgress]
}

export function useGeminiCliSetup() {
  const status = useGeminiCliStatus()
  const versions = useAvailableGeminiVersions()
  const installMutation = useInstallGeminiCli()
  const [progress, resetProgress] = useGeminiInstallProgress()

  const install = (
    version: string,
    options?: { onSuccess?: () => void; onError?: (error: Error) => void }
  ) => {
    resetProgress()
    installMutation.mutate(version, {
      onSuccess: () => options?.onSuccess?.(),
      onError: error => options?.onError?.(error),
    })
  }

  return {
    status: status.data,
    isStatusLoading: status.isLoading,
    versions: versions.data ?? [],
    isVersionsLoading: versions.isFetching,
    isVersionsError: versions.isError,
    refetchVersions: versions.refetch,
    needsSetup: !status.isLoading && !status.data?.installed,
    isInstalling: installMutation.isPending,
    installError: installMutation.error,
    progress,
    install,
    refetchStatus: status.refetch,
  }
}
