import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { JobLogEntry } from '../JobLogEntry';
import type { JobProgressEvent } from '../../../types/api';

const event: JobProgressEvent = { job_id: 'fixture', timestamp: 1, status: 'running', progress: 25, message: 'Installing package' };
describe('Job log entries', () => {
  it('preserves original newlines, blank lines and indentation', () => {
    const message = 'First line\n  indented detail\n\nLast line';
    const html = renderToStaticMarkup(<JobLogEntry event={{ ...event, message }} />);
    expect(html).toContain(message);
    expect(html).toContain('whitespace-pre-wrap');
  });
  it('renders command-like and HTML output as plain text', () => {
    const html = renderToStaticMarkup(<JobLogEntry event={{ ...event, message: '<script>alert(1)</script>' }} />);
    expect(html).toContain('&lt;script&gt;');
    expect(html).not.toContain('<script>');
  });
  it('includes timestamp, status and progress when a message is absent', () => {
    const html = renderToStaticMarkup(<JobLogEntry event={{ ...event, message: null }} />);
    expect(html).toContain('<time');
    expect(html).toContain('running');
    expect(html).toContain('Progres 25%');
  });
});
