/**
 * Global test setup for Vitest + React Testing Library.
 *
 * This file runs before every test suite. It extends Vitest's expect
 * with DOM-specific matchers from jest-dom (toBeInTheDocument, toHaveTextContent, etc.).
 */
import '@testing-library/jest-dom';
