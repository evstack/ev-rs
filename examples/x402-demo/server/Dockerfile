FROM oven/bun:1.1-alpine

WORKDIR /app

# Install dependencies
COPY package.json bun.lock* ./
RUN bun install --frozen-lockfile

# Copy source
COPY . .

# Expose port
EXPOSE 3000

# Run server
CMD ["bun", "run", "src/index.ts"]
