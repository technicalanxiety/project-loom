---
description: TypeScript and React conventions for loom-dashboard
inclusion: fileMatch
fileMatchPattern: "loom-dashboard/**/*.{ts,tsx}"
---

# TypeScript/React Conventions — loom-dashboard

## Stack

- **React 18** with functional components only (no class components)
- **Vite 6** for build tooling
- **react-router-dom v6** for routing
- **TypeScript 5.6+** in strict mode

## Style Rules

- Functional components with arrow function syntax: `const MyComponent: React.FC<Props> = ({ ... }) => { ... }`
- Prefer named exports over default exports.
- All component props get an explicit `interface` or `type` definition.
- Use `/** JSDoc */` comments on all exported functions, components, types, and hooks.
- Prefer `const` over `let`. Never use `var`.
- Destructure props in function signature.

## State & Data

- Prefer React hooks (`useState`, `useEffect`, `useMemo`, `useCallback`).
- Custom hooks for shared logic — prefix with `use`.
- API calls go through `src/api/client.ts` — no fetch calls in components.
- Type all API responses with interfaces in `src/types/`.

## File Organization

- One component per file. File name matches component name.
- Colocate component-specific types, hooks, and styles.
- Shared types in `src/types/index.ts`.
- API layer in `src/api/`.

## Error Handling

- Use discriminated unions for API response types (success | error).
- Display user-friendly error messages. Log technical details to console.
- Never swallow caught errors silently.
