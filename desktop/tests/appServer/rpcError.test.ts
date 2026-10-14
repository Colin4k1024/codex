import { describe, expect, it } from 'vitest'
import { classifyRpcError } from '../../src/main/appServer/rpcError'

describe('classifyRpcError', () => {
  it('classifies shutdown admission -32600 reason=serverShuttingDown', () => {
    const error = classifyRpcError({
      code: -32600,
      message: 'Server is draining; retry after reconnecting',
      data: { reason: 'serverShuttingDown' },
    })

    expect(error).toEqual({
      code: -32600,
      message: 'Server is draining; retry after reconnecting',
      kind: 'serverShuttingDown',
      data: { reason: 'serverShuttingDown' },
      retryable: false,
    })
  })

  it('keeps generic -32600 separate from shutdown admission', () => {
    expect(classifyRpcError({ code: -32600, message: 'bad request' }).kind).toBe('invalidRequest')
    expect(
      classifyRpcError({ code: -32600, message: 'other reason', data: { reason: 'other' } }).kind,
    ).toBe('invalidRequest')
  })

  it('classifies protocol error codes', () => {
    expect(classifyRpcError({ code: -32700, message: 'parse' }).kind).toBe('parse')
    expect(classifyRpcError({ code: -32601, message: 'unknown method' }).kind).toBe(
      'methodNotFound',
    )
    expect(classifyRpcError({ code: -32602, message: 'bad params' }).kind).toBe('invalidParams')
    expect(classifyRpcError({ code: -32603, message: 'boom' }).kind).toBe('internal')
  })

  it('marks overloaded -32001 as retryable', () => {
    const error = classifyRpcError({ code: -32001, message: 'Server overloaded; retry later.' })
    expect(error.kind).toBe('overloaded')
    expect(error.retryable).toBe(true)
  })

  it('falls back to unknown for unrecognized codes', () => {
    const error = classifyRpcError({ code: -32050, message: 'custom' })
    expect(error.kind).toBe('unknown')
    expect(error.retryable).toBe(false)
  })
})
