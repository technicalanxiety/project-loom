/**
 * Unit tests for the Predicates page component.
 *
 * Verifies rendering of candidates and packs, and resolution actions.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { BrowserRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';
import type { PackSummary, PredicateCandidateSummary } from '../types';
import { PredicatesPage } from './PredicatesPage';

vi.mock('../api/client', () => ({
  getPredicateCandidates: vi.fn(),
  getPredicatePacks: vi.fn(),
  resolvePredicateCandidate: vi.fn(),
}));

import {
  getPredicateCandidates,
  getPredicatePacks,
  resolvePredicateCandidate,
} from '../api/client';

const mockCandidates: PredicateCandidateSummary[] = [
  {
    id: 'pc1',
    predicate: 'connects_to',
    occurrences: 7,
    example_facts: ['f1', 'f2'],
    mapped_to: null,
    promoted_to_pack: null,
    created_at: '2025-06-01T00:00:00Z',
    resolved_at: null,
  },
  {
    id: 'pc2',
    predicate: 'tested_by',
    occurrences: 3,
    example_facts: ['f3'],
    mapped_to: 'uses',
    promoted_to_pack: null,
    created_at: '2025-05-20T00:00:00Z',
    resolved_at: '2025-05-25T00:00:00Z',
  },
];

const mockPacks: PackSummary[] = [
  { pack: 'core', description: 'Core predicates', predicate_count: 25 },
  { pack: 'grc', description: 'GRC predicates', predicate_count: 23 },
];

describe('PredicatesPage', () => {
  it('renders candidates and packs', async () => {
    vi.mocked(getPredicateCandidates).mockResolvedValueOnce(mockCandidates);
    vi.mocked(getPredicatePacks).mockResolvedValueOnce(mockPacks);

    render(
      <BrowserRouter>
        <PredicatesPage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('connects_to')).toBeInTheDocument();
    });

    expect(screen.getByText('7')).toBeInTheDocument();
    expect(screen.getByText(/Mapped → uses/)).toBeInTheDocument();
    expect(screen.getByText('core')).toBeInTheDocument();
    expect(screen.getByText('25 predicates')).toBeInTheDocument();
    expect(screen.getByText(/1 candidates pending review/)).toBeInTheDocument();
  });

  it('shows Map and Promote buttons for unresolved candidates', async () => {
    vi.mocked(getPredicateCandidates).mockResolvedValueOnce([mockCandidates[0]]);
    vi.mocked(getPredicatePacks).mockResolvedValueOnce([]);

    render(
      <BrowserRouter>
        <PredicatesPage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('Map')).toBeInTheDocument();
    });

    expect(screen.getByText('Promote')).toBeInTheDocument();
  });

  it('shows promote form when Promote is clicked', async () => {
    vi.mocked(getPredicateCandidates).mockResolvedValueOnce([mockCandidates[0]]);
    vi.mocked(getPredicatePacks).mockResolvedValueOnce([]);

    const user = userEvent.setup();

    render(
      <BrowserRouter>
        <PredicatesPage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('Promote')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Promote'));

    // Should show pack and category selects
    expect(screen.getByText('Confirm')).toBeInTheDocument();
    expect(screen.getByText('Cancel')).toBeInTheDocument();
  });

  it('calls resolvePredicateCandidate on promote confirm', async () => {
    vi.mocked(getPredicateCandidates)
      .mockResolvedValueOnce([mockCandidates[0]])
      .mockResolvedValueOnce([]);
    vi.mocked(getPredicatePacks).mockResolvedValue([]);
    vi.mocked(resolvePredicateCandidate).mockResolvedValueOnce({
      id: 'pc1',
      predicate: 'connects_to',
      action: 'promote',
      mapped_to: null,
      promoted_to_pack: 'core',
      resolved_at: '2025-06-02T00:00:00Z',
    });

    const user = userEvent.setup();

    render(
      <BrowserRouter>
        <PredicatesPage />
      </BrowserRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText('Promote')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Promote'));
    await user.click(screen.getByText('Confirm'));

    await waitFor(() => {
      expect(resolvePredicateCandidate).toHaveBeenCalledWith('pc1', {
        action: 'promote',
        target_pack: 'core',
        category: 'structural',
      });
    });
  });
});
