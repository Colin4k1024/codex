import { ConversationStream } from './ConversationStream'
import { ThreadList } from './ThreadList'
import type { ThreadWorkspaceController } from './useThreadWorkspace'

export interface ThreadsWorkspaceProps {
  controller: ThreadWorkspaceController
}

export function ThreadsWorkspace({ controller }: ThreadsWorkspaceProps) {
  return (
    <div className="grid gap-6 lg:grid-cols-[20rem_minmax(0,1fr)]">
      <ThreadList
        state={controller.state.list}
        onCreateThread={() => void controller.createThread()}
        onResumeThread={(threadId) => void controller.resumeThread(threadId)}
        onLoadNextPage={() => void controller.loadNextPage()}
      />
      <ConversationStream
        state={controller.state.view}
        onSubmit={(text) => void controller.startTurn(text)}
      />
    </div>
  )
}
