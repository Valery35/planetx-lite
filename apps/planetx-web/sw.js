// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Service worker страницы: страница работает без сети.
//
// - Своя страница (index.html, js, wasm, значки) - сначала сеть, без
//   сети - кэш. Новая сборка заменяет файлы прежней.
// - Тайлы - как дисковый кэш программы: тайл моложе 7 дней берётся
//   из кэша, старше - из сети, без сети - из кэша.
// - Прочие файлы (сцены по адресу, демо, шрифты, картинки меток) -
//   сначала сеть, без сети - кэш.
// - Поиск места не кэшируется.

const SHELL = 'planetx-shell';
const TILES = 'planetx-tiles';
const FILES = 'planetx-files';
const ALL = [SHELL, TILES, FILES];

// Свежесть тайла - net::FRESH программы.
const FRESH_MS = 7 * 24 * 3600 * 1000;
// Количество записей кэша, сверх него стираются самые старые.
const TILE_LIMIT = 20000;
const FILE_LIMIT = 500;
// Подрезка - не на каждую запись, а раз в столько записей.
const TRIM_EVERY = 200;
const STORED = 'x-planetx-stored';

const TILE_PREFIXES = [
  'https://server.arcgisonline.com/',
  'https://tile.openstreetmap.org/',
  'https://tiles.openfreemap.org/',
  'https://s3.amazonaws.com/elevation-tiles-prod/',
];
const NO_CACHE_PREFIXES = ['https://nominatim.openstreetmap.org/'];

// Файлы сборки Trunk с хэшем в имени.
const BUILT = /planetx-web-[0-9a-f]+(?:_bg\.wasm|\.js)/g;

self.addEventListener('install', (event) => {
  event.waitUntil(cacheShell().catch(() => undefined).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((names) => Promise.all(names.filter((n) => n.startsWith('planetx-') && !ALL.includes(n)).map((n) => caches.delete(n))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET' || request.cache === 'only-if-cached') {
    return;
  }
  const url = request.url;
  if (NO_CACHE_PREFIXES.some((p) => url.startsWith(p))) {
    return;
  }
  if (request.mode === 'navigate') {
    event.respondWith(page(event));
  } else if (new URL(url).origin === self.location.origin) {
    event.respondWith(networkFirst(event, SHELL, 0));
  } else if (TILE_PREFIXES.some((p) => url.startsWith(p))) {
    event.respondWith(tile(event));
  } else if (url.startsWith('https://')) {
    event.respondWith(networkFirst(event, FILES, FILE_LIMIT));
  }
});

// Страница и файлы сборки, на которые она ссылается. Файлы прежних
// сборок стираются.
async function cacheShell() {
  const index = new URL('./', self.registration.scope).href;
  const response = await fetch(index, { cache: 'no-cache' });
  if (!response.ok) {
    return;
  }
  const text = await response.clone().text();
  const cache = await caches.open(SHELL);
  await cache.put(index, response);
  const built = new Set((text.match(BUILT) || []).map((name) => new URL(name, index).href));
  for (const href of built) {
    if (!(await cache.match(href))) {
      const file = await fetch(href);
      if (file.ok) {
        await cache.put(href, file);
      }
    }
  }
  for (const key of await cache.keys()) {
    if (key.url.match(BUILT) && !built.has(key.url)) {
      await cache.delete(key);
    }
  }
}

// Страница по любому адресу с параметрами (?scene=...) - одна запись
// index.html.
async function page(event) {
  try {
    const response = await fetch(event.request);
    if (response.ok) {
      // Новая сборка: страница и её файлы - в кэш, прежние - вон.
      event.waitUntil(cacheShell().catch(() => undefined));
    }
    return response;
  } catch (error) {
    const cache = await caches.open(SHELL);
    const hit = await cache.match(new URL('./', self.registration.scope).href);
    if (hit) {
      return hit;
    }
    throw error;
  }
}

// Ответ уходит странице сразу, запись в кэш идёт следом: wasm
// компилируется по мере загрузки, а не после записи.
async function networkFirst(event, name, limit) {
  const request = event.request;
  const cache = await caches.open(name);
  try {
    const response = await fetch(request);
    if (response.ok && response.type !== 'opaque') {
      event.waitUntil(store(name, cache, request, response.clone(), limit).catch(() => undefined));
    }
    return response;
  } catch (error) {
    const hit = await cache.match(request);
    if (hit) {
      return hit;
    }
    throw error;
  }
}

async function tile(event) {
  const request = event.request;
  const cache = await caches.open(TILES);
  const hit = await cache.match(request);
  if (hit && Date.now() - Number(hit.headers.get(STORED) || 0) < FRESH_MS) {
    return hit;
  }
  try {
    const response = await fetch(request);
    if (response.ok) {
      event.waitUntil(store(TILES, cache, request, response.clone(), TILE_LIMIT).catch(() => undefined));
      return response;
    }
    // 404 - тайла нет, это ответ. Ошибка сервера - лучше старый тайл.
    return response.status !== 404 && hit ? hit : response;
  } catch (error) {
    if (hit) {
      return hit;
    }
    throw error;
  }
}

// Запись с временем сохранения. Ответ CORS не отдаёт заголовок Date,
// поэтому время - свой заголовок.
const written = new Map();
async function store(name, cache, request, response, limit) {
  const headers = new Headers(response.headers);
  headers.set(STORED, String(Date.now()));
  const body = await response.arrayBuffer();
  await cache.put(request, new Response(body, { status: response.status, statusText: response.statusText, headers }));
  if (limit > 0) {
    const count = (written.get(name) || 0) + 1;
    written.set(name, count);
    if (count % TRIM_EVERY === 1) {
      await trim(cache, limit);
    }
  }
}

// Записи кэша идут в порядке записи - стираются самые старые.
async function trim(cache, limit) {
  const keys = await cache.keys();
  const extra = keys.length - limit;
  for (let i = 0; i < extra; i += 1) {
    await cache.delete(keys[i]);
  }
}
