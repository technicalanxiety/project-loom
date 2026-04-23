# Build stage
FROM node:22-slim AS builder
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm install
COPY . .
RUN npm run build

# Output stage — copies static files into the dashboard_dist named volume
# on every startup so new image layers are always reflected after a pull.
# Docker only auto-populates named volumes on first creation; the explicit
# cp here ensures updates land even when the volume already exists.
FROM busybox:1.37
COPY --from=builder /app/dist /build/dist
CMD ["sh", "-c", "cp -r /build/dist/. /app/dist/"]
