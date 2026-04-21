/**
 * Unit tests for the Conflicts page component.
 *
 * Verifies rendering of conflict list and resolution actions.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { BrowserRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';
import type { ConflictSummary } from '../types';
import { ConflictsPage } from './ConflictsPage';

vi.mock('../api/client', () => ({
  getConflicts: vi.fn(),
  resolveConflict: vi.fn(),
}));

import { getConflicts, resolveConflict } from '../api/client';

const mockConflicts: ConflictSummary[] = [
  {
    id: 'c1',
    entity_name: 'APIM',
    entity_type: 'service',
    namespace: 'default',
    candidates: [{ name: 'Azure API Management', score: 0.93 }],
    resolved: false,
    resolution: null,
    created_at: '2025-06-01T00:00:00Z',
  },
  {
    id: 'c2',
    entity_name: 'Redis',
    entity_type: 'technology',
    namespace: 'default',
    candidates: null,
    resolved: true,
    resolution: 'kept_separate',
    created_at: '2025-05-15T00:00:00Z',
  },
];

describe('ConflictsPage', () => {
  it('renders conflict list with unresolved and resolved items', async () => {
    vi.mocked(getConflicts).mockResolvedValueOnce(mockConflicts);

    render(
      <BrowserRouter>
        <ConflictsPage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('APIM')).toBeInTheDocument();
    });

    expect(screen.getByText('Redis')).toBeInTheDocument();
    expect(screen.getByText('kept_separate')).toBeInTheDocument();
    expect(screen.getByText(/1 unresolved of 2 total/)).toBeInTheDocument();
  });

  it('shows resolution action buttons for unresolved conflicts', async () => {
    vi.mocked(getConflicts).mockResolvedValueOnce([mockConflicts[0]]);

    render(
      <BrowserRouter>
        <ConflictsPage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('Keep Separate')).toBeInTheDocument();
    });

    expect(screen.getByText('Merge')).toBeInTheDocument();
    expect(screen.getByText('Split')).toBeInTheDocument();
  });

  it('calls resolveConflict when action button is clicked', async () => {
    vi.mocked(getConflicts)
      .mockResolvedValueOnce([mockConflicts[0]])
      .mockResolvedValueOnce([
        { ...mockConflicts[0], resolved: true, resolution: 'kept_separate' },
      ]);
    vi.mocked(resolveConflict).mockResolvedValueOnce({
      id: 'c1',
      resolved: true,
      resolution: 'kept_separate',
      resolved_at: '2025-06-02T00:00:00Z',
    });

    const user = userEvent.setup();

    render(
      <BrowserRouter>
        <ConflictsPage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('Keep Separate')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Keep Separate'));

    await waitFor(() => {
      expect(resolveConflict).toHaveBeenCalledWith('c1', { resolution: 'kept_separate' });
    });
  });

  it('shows empty state when no conflicts', async () => {
    vi.mocked(getConflicts).mockResolvedValueOnce([]);

    render(
      <BrowserRouter>
        <ConflictsPage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('No conflicts found.')).toBeInTheDocument();
    });
  });
});
