FROM oven/bun:1.1-alpine

WORKDIR /app

# Install dependencies
COPY package.json bun.lock* ./
RUN bun install --frozen-lockfile

# Copy source
COPY . .

# Expose port
EXPOSE 5173

# Run dev server (use --host to allow external connections)
CMD ["bun", "run", "dev", "--", "--host", "0.0.0.0"]
