/**
 * Unit tests for the Pipeline Health (HomePage) component.
 *
 * Mocks the API client to verify rendering with sample data.
 */
import { render, screen, waitFor } from '@testing-library/react';
import { BrowserRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';
import type { PipelineHealthResponse } from '../types';
import { HomePage } from './HomePage';

vi.mock('../api/client', () => ({
  getPipelineHealth: vi.fn(),
}));

import { getPipelineHealth } from '../api/client';

const mockHealth: PipelineHealthResponse = {
  episodes_by_source: [
    { key: 'claude-code', count: 42 },
    { key: 'manual', count: 8 },
  ],
  episodes_by_namespace: [{ key: 'default', count: 50 }],
  entities_by_type: [
    { key: 'service', count: 15 },
    { key: 'person', count: 10 },
  ],
  facts_current: 120,
  facts_superseded: 5,
  queue_depth: 3,
  extraction_model: 'gemma4:26b-a4b-q4',
  classification_model: 'gemma4:e4b',
};

describe('HomePage', () => {
  it('renders pipeline health data', async () => {
    vi.mocked(getPipelineHealth).mockResolvedValueOnce(mockHealth);

    render(
      <BrowserRouter>
        <HomePage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('120')).toBeInTheDocument();
    });

    expect(screen.getByText('5')).toBeInTheDocument();
    expect(screen.getByText('3')).toBeInTheDocument();
    expect(screen.getByText('gemma4:26b-a4b-q4')).toBeInTheDocument();
    expect(screen.getByText('gemma4:e4b')).toBeInTheDocument();
    expect(screen.getByText('claude-code')).toBeInTheDocument();
    expect(screen.getByText('42')).toBeInTheDocument();
  });

  it('shows loading state', () => {
    vi.mocked(getPipelineHealth).mockReturnValueOnce(new Promise(() => {}));

    render(
      <BrowserRouter>
        <HomePage />
      </BrowserRouter>,
    );

    expect(screen.getByText(/loading/i)).toBeInTheDocument();
  });

  it('shows error state', async () => {
    vi.mocked(getPipelineHealth).mockRejectedValueOnce(new Error('Network error'));

    render(
      <BrowserRouter>
        <HomePage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('Network error')).toBeInTheDocument();
    });
  });
});
