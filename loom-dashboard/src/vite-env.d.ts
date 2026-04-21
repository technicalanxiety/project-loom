/// <reference types="vite/client" />

// Ambient declarations for non-JS assets imported as side effects or value.
// Vite handles these at build time; TypeScript needs explicit module
// declarations for them since TS 6 no longer permits untyped side-effect
// imports.

declare module '*.css';
declare module '*.svg' {
  const src: string;
  export default src;
}
declare module '*.png' {
  const src: string;
  export default src;
}
