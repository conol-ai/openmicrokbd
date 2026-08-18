import type { Plugin } from "@opencode-ai/plugin"

const OPENMICRO = "/Applications/OpenMicro.app/Contents/MacOS/OpenMicro"

type StatusEvent = {
  type: string
  properties?: {
    sessionID?: string
    status?: { type?: string }
    error?: { name?: string }
    info?: {
      // Current session.deleted events carry a Session object as `info`.
      // Accept both names to remain compatible across OpenCode SDK versions.
      id?: string
      sessionID?: string
    }
  }
}

/**
 * Install as ~/.config/opencode/plugins/openmicro.ts (or in
 * .opencode/plugins/openmicro.ts for one project). Source builds should change
 * OPENMICRO above to the absolute path of their binary.
 */
export const OpenMicroStatus: Plugin = async ({ $ }) => {
  // OpenCode emits session.error immediately before idle in some failure
  // paths. Let OpenMicro's short error TTL own that transition instead of
  // replacing red with green immediately.
  const suppressNextIdleSuccess = new Set<string>()
  // Current OpenCode emits both session.status and the deprecated
  // session.idle. Once this runtime has emitted the modern event, ignore its
  // duplicate so an error-suppressed idle cannot turn red into green.
  let usesSessionStatus = false
  // OpenCode does not await plugin event handlers. Serialize helper processes
  // here so a slow working update cannot arrive after a newer approval/error.
  let sendQueue = Promise.resolve()

  const send = async (status: string, sessionID: string) => {
    try {
      await $`${OPENMICRO} status ${status} ${`opencode:${sessionID}`}`.quiet()
    } catch {
      // The light bridge is best-effort and must never interrupt the agent.
    }
  }

  const enqueue = (status: string, sessionID: string) => {
    sendQueue = sendQueue.then(() => send(status, sessionID))
    return sendQueue
  }

  return {
    event: async ({ event }) => {
      const { type, properties = {} } = event as StatusEvent
      const sessionID =
        properties.sessionID ??
        properties.info?.sessionID ??
        properties.info?.id ??
        "default"
      const status = properties.status?.type

      if (type === "session.status") {
        usesSessionStatus = true
        if (status === "busy" || status === "retry") {
          suppressNextIdleSuccess.delete(sessionID)
          await enqueue("working", sessionID)
        } else if (
          status === "idle" &&
          !suppressNextIdleSuccess.delete(sessionID)
        ) {
          await enqueue("success", sessionID)
        }
      } else if (
        type === "session.idle" &&
        !usesSessionStatus &&
        !suppressNextIdleSuccess.delete(sessionID)
      ) {
        // Compatibility with OpenCode versions from before session.status.
        await enqueue("success", sessionID)
      } else if (
        type === "permission.asked" ||
        type === "permission.updated" ||
        type === "question.asked"
      ) {
        await enqueue("attention", sessionID)
      } else if (
        type === "permission.replied" ||
        type === "question.replied" ||
        type === "question.rejected"
      ) {
        suppressNextIdleSuccess.delete(sessionID)
        await enqueue("working", sessionID)
      } else if (type === "session.error") {
        suppressNextIdleSuccess.add(sessionID)
        await enqueue(
          properties.error?.name === "MessageAbortedError" ? "idle" : "error",
          sessionID,
        )
      } else if (type === "session.deleted") {
        suppressNextIdleSuccess.delete(sessionID)
        await enqueue("idle", sessionID)
      }
    },
  }
}
