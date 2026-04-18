# Build stage
FROM node:20-slim AS builder
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm install
COPY . .
RUN npm run build

# Output stage — static files only
FROM scratch
COPY --from=builder /app/dist /app/dist
