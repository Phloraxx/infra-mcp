FROM node:22-alpine AS build
WORKDIR /repo

COPY package.json tsconfig.base.json ./
COPY apps/mcp-server/package.json ./apps/mcp-server/package.json
RUN npm install --workspace @infra-mcp/mcp-server

COPY apps/mcp-server ./apps/mcp-server
RUN npm run build --workspace @infra-mcp/mcp-server

FROM node:22-alpine AS runtime
WORKDIR /app
ENV NODE_ENV=production

COPY apps/mcp-server/package.json ./package.json
RUN npm install --omit=dev && npm cache clean --force
COPY --from=build /repo/apps/mcp-server/dist ./dist

USER node
EXPOSE 3000
CMD ["node", "dist/index.js"]
