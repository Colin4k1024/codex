import { describe, expect, expectTypeOf, it } from 'vitest'
import type {
  ClientRequest,
  InitializeParams,
  InitializeResponse,
  JSONRPCError,
  JSONRPCErrorError,
  JSONRPCMessage,
  JSONRPCNotification,
  JSONRPCRequest,
  JSONRPCResponse,
  ServerNotification,
  V2CommandExecutionRequestApprovalParams,
  V2CommandExecutionRequestApprovalResponse,
  V2FileChangeRequestApprovalParams,
  V2FileChangeRequestApprovalResponse,
  V2PermissionsRequestApprovalParams,
  V2PermissionsRequestApprovalResponse,
  V2ThreadListParams,
  V2ThreadListResponse,
  V2ThreadResumeParams,
  V2ThreadResumeResponse,
  V2ThreadStartParams,
  V2ThreadStartResponse,
  V2TurnStartParams,
  V2TurnStartResponse,
} from '../src'
import { PROTOCOL_FILE_COUNT, PROTOCOL_REVISION } from '../src'

describe('generated app-server protocol', () => {
  it('exports a stable revision and file count', () => {
    expect(PROTOCOL_REVISION).toMatch(/^[a-f0-9]{64}$/)
    expect(PROTOCOL_FILE_COUNT).toBeGreaterThan(700)
  })

  it('compiles the core JSON-RPC envelope contracts', () => {
    expectTypeOf<JSONRPCMessage>().not.toBeAny()
    expectTypeOf<JSONRPCRequest>().not.toBeAny()
    expectTypeOf<JSONRPCResponse>().not.toBeAny()
    expectTypeOf<JSONRPCNotification>().not.toBeAny()
    expectTypeOf<JSONRPCError>().not.toBeAny()
    expectTypeOf<JSONRPCErrorError>().not.toBeAny()
  })

  it('compiles thread and turn request/response contracts', () => {
    expectTypeOf<InitializeParams>().not.toBeAny()
    expectTypeOf<InitializeResponse>().not.toBeAny()
    expectTypeOf<ClientRequest>().not.toBeAny()
    expectTypeOf<ServerNotification>().not.toBeAny()
    expectTypeOf<V2ThreadStartParams>().not.toBeAny()
    expectTypeOf<V2ThreadStartResponse>().not.toBeAny()
    expectTypeOf<V2ThreadResumeParams>().not.toBeAny()
    expectTypeOf<V2ThreadResumeResponse>().not.toBeAny()
    expectTypeOf<V2ThreadListParams>().not.toBeAny()
    expectTypeOf<V2ThreadListResponse>().not.toBeAny()
    expectTypeOf<V2TurnStartParams>().not.toBeAny()
    expectTypeOf<V2TurnStartResponse>().not.toBeAny()
  })

  it('compiles command, file, and permissions approval contracts', () => {
    expectTypeOf<V2CommandExecutionRequestApprovalParams>().not.toBeAny()
    expectTypeOf<V2CommandExecutionRequestApprovalResponse>().not.toBeAny()
    expectTypeOf<V2FileChangeRequestApprovalParams>().not.toBeAny()
    expectTypeOf<V2FileChangeRequestApprovalResponse>().not.toBeAny()
    expectTypeOf<V2PermissionsRequestApprovalParams>().not.toBeAny()
    expectTypeOf<V2PermissionsRequestApprovalResponse>().not.toBeAny()
  })
})

describe('generated app-server protocol invariants', () => {
  it('rejects malformed thread and turn payloads at compile time', () => {
    // @ts-expect-error cwd must be a string or null.
    const invalidThreadStart: V2ThreadStartParams = { cwd: 123 }

    // @ts-expect-error limit must be a number or null.
    const invalidThreadList: V2ThreadListParams = { limit: '10' }

    // @ts-expect-error threadId must be a string.
    const invalidTurnStart: V2TurnStartParams = { threadId: 123, input: [] }

    expect(invalidThreadStart).toBeDefined()
    expect(invalidThreadList).toBeDefined()
    expect(invalidTurnStart).toBeDefined()
  })

  it('rejects malformed JSON-RPC envelopes at compile time', () => {
    // @ts-expect-error id is required.
    const invalidRequest: JSONRPCRequest = { method: 'initialize' }

    // @ts-expect-error method must be a known client request method.
    const invalidClientRequest: ClientRequest = { method: 'unknown/method', id: '1' }

    // @ts-expect-error id is required on responses.
    const invalidResponseId: JSONRPCResponse = { result: {} }

    // @ts-expect-error result is required.
    const invalidResponse: JSONRPCResponse = { id: '1' }

    // @ts-expect-error error is required.
    const invalidError: JSONRPCError = { id: '1' }

    // @ts-expect-error code must be a number.
    const invalidErrorDetails: JSONRPCErrorError = { code: '1', message: 'error' }

    expect(invalidRequest).toBeDefined()
    expect(invalidClientRequest).toBeDefined()
    expect(invalidResponseId).toBeDefined()
    expect(invalidResponse).toBeDefined()
    expect(invalidError).toBeDefined()
    expect(invalidErrorDetails).toBeDefined()
  })
})
