FROM node:20-alpine as build
COPY packages/api/package.json /app/package.json
COPY packages/api/package-lock.json /app/package-lock.json
COPY packages/indexer/migrations /app/migrations
COPY packages/api/test /app/test
COPY packages/api/src /app/src
COPY packages/api/data_contracts /app/data_contracts
COPY packages/api/.env /app/.env
COPY packages/api/index.js /app/index.js
WORKDIR /app

RUN apk add git
RUN npm ci

FROM node:20-alpine as api
WORKDIR /app
COPY --from=build /app /app
CMD ["npm", "start"]
