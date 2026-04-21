# Build stage
FROM node:22-slim AS builder
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm install
COPY . .
RUN npm run build

# Output stage — static files only, used as a data volume source.
# busybox provides a minimal filesystem so Docker can populate the
# named volume from the image layer on first run.
FROM busybox:1.37
COPY --from=builder /app/dist /app/dist
# No CMD — container exits immediately after volume population.
