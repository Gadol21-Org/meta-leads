# syntax=docker/dockerfile:1
FROM node:22-alpine
ENV NODE_ENV=production DATA_FILE=/data/db.json PORT=3000
WORKDIR /app
COPY package*.json ./
RUN npm ci --omit=dev
COPY src src
COPY public public
# The JSON store lives in /data, a Docker volume owned by the unprivileged user.
RUN mkdir /data && chown node:node /data
USER node
VOLUME /data
EXPOSE 3000
CMD ["node", "src/index.js"]
