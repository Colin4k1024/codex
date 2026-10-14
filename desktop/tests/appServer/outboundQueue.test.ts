import { EventEmitter } from 'node:events'
import { describe, expect, it, vi } from 'vitest'
import { OutboundQueue } from '../../src/main/appServer/outboundQueue'

class FakeWritable extends EventEmitter {
  written: string[] = []
  backpressure = false

  write(frame: string): boolean {
    this.written.push(frame)
    return !this.backpressure
  }
}

function createQueue(stream: FakeWritable, maxQueuedMessages: number) {
  const onOverflow = vi.fn()
  const queue = new OutboundQueue({ stream, maxQueuedMessages, onOverflow })
  return { queue, onOverflow }
}

describe('OutboundQueue', () => {
  it('preserves FIFO order for requests and notifications', () => {
    const stream = new FakeWritable()
    const { queue } = createQueue(stream, 10)

    queue.push('one\n')
    queue.push('two\n')
    queue.push('three\n')

    expect(stream.written).toEqual(['one\n', 'two\n', 'three\n'])
  })

  it('waits for drain instead of dropping or reordering frames', () => {
    const stream = new FakeWritable()
    stream.backpressure = true
    const { queue } = createQueue(stream, 10)

    queue.push('one\n')
    queue.push('two\n')
    queue.push('three\n')

    expect(stream.written).toEqual(['one\n'])
    expect(queue.length).toBe(2)

    stream.backpressure = false
    stream.emit('drain')

    expect(stream.written).toEqual(['one\n', 'two\n', 'three\n'])
    expect(queue.length).toBe(0)
  })

  it('fails deterministically when the outstanding-message bound is exceeded', () => {
    const stream = new FakeWritable()
    stream.backpressure = true
    const { queue, onOverflow } = createQueue(stream, 3)

    // The first frame is written immediately; the stream then applies
    // backpressure, so the next three frames stay outstanding.
    queue.push('one\n')
    queue.push('two\n')
    queue.push('three\n')
    expect(queue.length).toBe(2)

    queue.push('four\n')
    queue.push('five\n')

    expect(onOverflow).toHaveBeenCalledTimes(1)
    expect(onOverflow.mock.calls[0][0]).toMatchObject({ kind: 'capacity' })
    expect(queue.length).toBe(0)

    queue.push('six\n')
    expect(stream.written).toEqual(['one\n'])
  })
})
