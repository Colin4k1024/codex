import { describe, expect, it } from 'vitest'
import { JsonRpcClient } from '../../src/main/appServer/jsonRpcClient'

function createClient(options: Partial<ConstructorParameters<typeof JsonRpcClient>[0]> = {}) {
  const sent: string[] = []
  const notifications: Array<{ method: string; params: unknown }> = []
  const serverRequests: Array<{ id: string | number; method: string; params?: unknown }> = []
  const protocolErrors: unknown[] = []
  const client = new JsonRpcClient({
    send: (frame) => {
      sent.push(frame)
    },
    onNotification: (method, params) => {
      notifications.push({ method, params })
    },
    onServerRequest: (request) => {
      serverRequests.push(request)
    },
    onProtocolError: (error) => {
      protocolErrors.push(error)
    },
    ...options,
  })
  return { client, sent, notifications, serverRequests, protocolErrors }
}

describe('JsonRpcClient', () => {
  it('uses unique request ids and removes pending entries after responses', async () => {
    const { client, sent } = createClient()

    const first = client.request('account/logout')
    const second = client.request('account/logout')
    const third = client.request('account/logout')

    expect(client.pendingCount).toBe(3)
    const ids = sent.map((frame) => JSON.parse(frame).id)
    expect(new Set(ids).size).toBe(3)

    for (const id of ids) {
      client.feedLine(JSON.stringify({ id, result: { id } }))
    }

    expect(await Promise.all([first, second, third])).toEqual(ids.map((id) => ({ id })))
    expect(client.pendingCount).toBe(0)
  })

  it('maps error responses to classified RPC errors', async () => {
    const { client, sent } = createClient()
    const pending = client.request('thread/list', {})
    const id = JSON.parse(sent[0]).id

    client.feedLine(
      JSON.stringify({
        id,
        error: { code: -32001, message: 'Server overloaded; retry later.' },
      }),
    )

    await expect(pending).rejects.toMatchObject({ kind: 'overloaded', retryable: true })
    expect(client.pendingCount).toBe(0)
  })

  it('rejects requests beyond the pending limit', async () => {
    const { client } = createClient({ limits: { maxPendingRequests: 2 } })

    client.request('account/logout')
    client.request('account/logout')
    await expect(client.request('account/logout')).rejects.toMatchObject({ kind: 'capacity' })
  })

  it('rejects duplicate ids produced by a broken id factory', async () => {
    const { client } = createClient({ nextRequestId: () => 7 })

    client.request('account/logout')
    await expect(client.request('account/logout')).rejects.toMatchObject({ kind: 'internal' })
  })

  it('times out requests and clears their pending entries', async () => {
    const { client } = createClient({ limits: { requestTimeoutMs: 5 } })
    const pending = client.request('account/logout')

    await expect(pending).rejects.toMatchObject({ kind: 'timeout', retryable: true })
    expect(client.pendingCount).toBe(0)
  })

  it('reports malformed JSON frames as protocol errors', () => {
    const { client, protocolErrors, notifications } = createClient()

    client.feedLine('this is not json')
    client.feedLine(JSON.stringify({ method: 'thread/started', params: { id: 'thread-1' } }))

    expect(protocolErrors).toHaveLength(1)
    expect(protocolErrors[0]).toMatchObject({ kind: 'parse', code: -32700 })
    expect(notifications).toEqual([{ method: 'thread/started', params: { id: 'thread-1' } }])
  })

  it('invokes notification callbacks in wire order', () => {
    const { client, notifications } = createClient()

    for (let sequence = 0; sequence < 5; sequence += 1) {
      client.feedLine(JSON.stringify({ method: 'test/ordered', params: { sequence } }))
    }

    expect(notifications.map((event) => (event.params as { sequence: number }).sequence)).toEqual([
      0, 1, 2, 3, 4,
    ])
  })

  it('captures server requests and responds with the original id', () => {
    const { client, sent, serverRequests, notifications } = createClient()

    client.feedLine(
      JSON.stringify({
        id: 'approval-1',
        method: 'item/commandExecution/requestApproval',
        params: { command: 'echo hello' },
      }),
    )
    client.respondToServerRequest('approval-1', { decision: 'decline' })

    expect(serverRequests).toEqual([
      {
        id: 'approval-1',
        method: 'item/commandExecution/requestApproval',
        params: { command: 'echo hello' },
      },
    ])
    expect(notifications).toEqual([])
    expect(JSON.parse(sent[0])).toEqual({
      id: 'approval-1',
      result: { decision: 'decline' },
    })
  })

  it('rejects duplicate active server request ids and unknown responses', () => {
    const { client, sent, protocolErrors } = createClient()
    const frame = JSON.stringify({
      id: 42,
      method: 'item/fileChange/requestApproval',
      params: {},
    })

    client.feedLine(frame)
    client.feedLine(frame)

    expect(protocolErrors).toHaveLength(1)
    expect(protocolErrors[0]).toMatchObject({ kind: 'parse' })
    expect(() => client.respondToServerRequest(999, null)).toThrowError(/not active/)

    client.respondToServerRequest(42, { decision: 'decline' })
    expect(JSON.parse(sent[0])).toEqual({ id: 42, result: { decision: 'decline' } })
    expect(() => client.respondToServerRequest(42, null)).toThrowError(/not active/)
  })

  it('forgets server requests when the server reports them resolved', () => {
    const { client } = createClient()
    client.feedLine(
      JSON.stringify({
        id: 7,
        method: 'item/tool/requestUserInput',
        params: {},
      }),
    )
    client.forgetServerRequest(7)
    expect(() => client.respondToServerRequest(7, {})).toThrowError(/not active/)
  })

  it('drops late responses after disconnect', async () => {
    const { client, sent } = createClient()
    const pending = client.request('account/logout')
    const id = JSON.parse(sent[0]).id

    client.end()
    await expect(pending).rejects.toMatchObject({ kind: 'connectionClosed' })

    client.feedLine(JSON.stringify({ id, result: { late: true } }))
  })

  it('fails all pending requests on disconnect', async () => {
    const { client } = createClient()
    const pending = [client.request('account/logout'), client.request('account/logout')]

    client.end()

    await Promise.all(
      pending.map((promise) => expect(promise).rejects.toMatchObject({ kind: 'connectionClosed' })),
    )
    expect(client.pendingCount).toBe(0)
  })

  it('rejects locally invalid methods', async () => {
    const { client } = createClient()

    await expect(client.request('')).rejects.toMatchObject({ kind: 'invalidRequest', code: -32600 })
    expect(() => client.notify('')).toThrowError(/non-empty/)
  })

  it('ignores responses for unknown request ids', () => {
    const { client, protocolErrors } = createClient()

    client.feedLine(JSON.stringify({ id: 999, result: null }))

    expect(protocolErrors).toHaveLength(0)
  })

  it('rejects and reports response payloads with invalid ids', () => {
    const { client, protocolErrors } = createClient()

    client.feedLine(JSON.stringify({ id: 'not-a-number', result: null }))

    expect(protocolErrors).toHaveLength(1)
    expect(protocolErrors[0]).toMatchObject({ kind: 'parse' })
  })
})
