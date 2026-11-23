import { useEffect, useRef } from 'react';
import { Box, Text, Title, Tooltip } from '@mantine/core';
import { usePage } from '../pageContext';
import { usePanes } from '../paneContext';

type PaneReg = {
  container: HTMLElement;
  items: Map<number, HTMLElement>;
  handlerAttached: boolean;
  rafId: number | null;
  lastIndex: number | null;
  cleanupTimer: number | null;
};

const __paneRegMap: Map<number, PaneReg> = new Map();

interface passageSectionProps {
  html: string;
  index: number;
  paneId: number;
  tabId: number;
}

export function PassageSection({ html, index, paneId, tabId }: passageSectionProps) {
  const sectionRef = useRef<HTMLDivElement>(null);

  const { booksGrouped, getFullBookName } = usePage();
  const { panes, getActiveTab, updateActiveIndex } = usePanes();

  const pane = panes.find((p) => p.id === paneId) || null;
  const activeTab = getActiveTab(pane);
  const result = activeTab?.results[index];

  function findScrollViewport(start: HTMLElement | null): HTMLElement | null {
    let el: HTMLElement | null = start;
    while (el) {
      if (el.classList?.contains('mantine-ScrollArea-viewport')) return el;
      if (el.getAttribute?.('data-scroll-area-viewport')) return el;
      if (el.classList?.contains('m_c0783ff9')) return el;
      el = el.parentElement;
    }
    return null;
  }

  function buildFootnoteTooltipMap(tab: any): Map<string, string> {
    const map = new Map<string, string>();
    for (const r of tab.results) {
      const fns = (r.footnotes ?? []) as any[];
      for (const fn of fns) {
        let marker = '';
        const m = String(fn.noteId ?? '').match(/-(\w+)$/);
        if (m) marker = m[1].toLowerCase();
        if (!marker && /^[A-Za-z]$/.test(String(fn.noteId ?? '')))
          marker = String(fn.noteId).toLowerCase();
        if (!marker) continue;

        const rendered = String((fn as any).text ?? '');
        let content = '';
        const idxColon = rendered.indexOf(':');
        if (idxColon !== -1) content = rendered.slice(idxColon + 1).trim();
        else {
          const idxDash = rendered.indexOf(' - ');
          content = idxDash !== -1 ? rendered.slice(idxDash + 3).trim() : rendered.trim();
        }
        const key = `${fn.chapter}:${fn.verse}|${marker}`;
        map.set(key, content);
      }
    }
    return map;
  }

  function formatCrossrefLinksForTooltip(links: any): string {
    if (!links) return '';
    const toText = (l: any) => {
      const name = getFullBookName(l.book, booksGrouped);
      if (l?.verseStart !== undefined && l?.verseEnd !== undefined)
        return `${name} ${l.chapter}:${l.verseStart}-${l.verseEnd}`;
      if (l?.verse !== undefined) return `${name} ${l.chapter}:${l.verse}`;
      if (l?.chapter !== undefined) return `${name} ${l.chapter}`;
      return name;
    };
    if (Array.isArray(links)) return links.map(toText).join('; ');
    if (typeof links === 'string') return links;
    return '';
  }

  function buildCrossrefTooltipMap(tab: any): Map<string, string> {
    const map = new Map<string, string>();
    for (const r of tab.results) {
      const crs = (r.crossrefs ?? []) as any[];
      for (const cr of crs) {
        const m = String(cr.refId ?? '').match(/-([A-Za-z]+)$/);
        const label = m ? m[1].toUpperCase() : '';
        if (!label) continue;
        const content = formatCrossrefLinksForTooltip(cr.links);
        const key = `${cr.chapter}:${cr.verse}|${label}`;
        map.set(key, content);
      }
    }
    return map;
  }

  function renderInlineWithTooltips(
    text: string,
    opts: {
      show_fn: boolean;
      show_cr: boolean;
      baseChapter: number;
      isThomas: boolean;
      fnTips: Map<string, string>;
      crTips: Map<string, string>;
    },
    state: { currentVerse: number },
    keyPrefix: string
  ) {
    const nodes: any[] = [];
    let lastIndex = 0;
    const re = /\[(vs|cr|fn|inline):([^\]]+)\]/g;
    let m: RegExpExecArray | null;
    let hasInline = false;

    while ((m = re.exec(text))) {
      const [full, kind, val] = m;
      if (m.index > lastIndex) nodes.push(text.slice(lastIndex, m.index));

      if (kind === 'vs') {
        state.currentVerse = parseInt(val, 10) || state.currentVerse;

        // Check if the next marker is an inline heading
        const remainingText = text.slice(m.index + full.length);
        const nextMarkerMatch = remainingText.match(/^\s*\[inline:/);

        if (nextMarkerMatch) {
          // Skip this verse marker because inline heading follows
          hasInline = true;
        } else if (opts.isThomas && !nextMarkerMatch && val === '0') {
          nodes.push(
            <span
              key={`${keyPrefix}-prologue-${m.index}`}
              style={{
                display: 'inline',
                fontSize: '2.0rem',
                fontWeight: 700,
                position: 'relative',
                lineHeight: '.8em',
                bottom: '-.1em',
                marginRight: '0.35em',
              }}
            >
              Prologue
            </span>
          );
        } else if (!opts.isThomas && !hasInline && val === '1') {
          const chapterLabel = opts.baseChapter ?? state.currentVerse;
          nodes.push(
            <span
              key={`${keyPrefix}-chapter-${chapterLabel}-${m.index}`}
              style={{
                display: 'inline',
                fontSize: '2.0rem',
                position: 'relative',
                lineHeight: '.8em',
                bottom: '-.1em',
              }}
            >
              {chapterLabel}
            </span>
          );
        } else if (!nextMarkerMatch) {
          nodes.push(<sup key={`${keyPrefix}-vs-${state.currentVerse}-${m.index}`}>{val}</sup>);
        }
      } else if (kind === 'cr') {
        if (opts.show_cr) {
          const label = String(val).toUpperCase();
          const tip = opts.crTips.get(`${opts.baseChapter}:${state.currentVerse}|${label}`) || '';
          nodes.push(
            <Tooltip key={`${keyPrefix}-cr-${label}-${m.index}`} label={tip} withArrow>
              <span className="marker marker-cr">[{label}]</span>
            </Tooltip>
          );
        } else {
          nodes.push(' ');
        }
      } else if (kind === 'fn') {
        if (opts.show_fn) {
          const label = String(val).toLowerCase();
          const tip = opts.fnTips.get(`${opts.baseChapter}:${state.currentVerse}|${label}`) || '';
          nodes.push(
            <Tooltip key={`${keyPrefix}-fn-${label}-${m.index}`} label={tip} withArrow>
              <span className="marker marker-fn">[{label}]</span>
            </Tooltip>
          );
        } else {
          nodes.push(' ');
        }
      } else if (kind === 'inline') {
        const inlineText = val.replace(/&#93;/g, ']');
        nodes.push(
          <Text fw={700} component="span" key={`${keyPrefix}-inline-${m.index}`}>
            {inlineText}
          </Text>
        );
        hasInline = true;

        // Check if the text immediately after the marker duplicates the inline text
        // If so, skip over it to avoid duplication
        const afterMarker = text.slice(m.index + full.length);
        if (afterMarker.trimStart().startsWith(inlineText)) {
          lastIndex = m.index + full.length + afterMarker.indexOf(inlineText) + inlineText.length;
          continue;
        }
      }
      lastIndex = m.index + full.length;
    }
    if (lastIndex < text.length) nodes.push(text.slice(lastIndex));
    return nodes;
  }

  function renderPassageReact(
    html: string,
    opts: {
      show_fn: boolean;
      show_cr: boolean;
      baseChapter: number;
      isThomas: boolean;
      fnTips: Map<string, string>;
      crTips: Map<string, string>;
    }
  ) {
    const blocks: any[] = [];
    let idx = 0;
    const re =
      /<(h3)([^>]*)>([\s\S]*?)<\/h3>|<(p)(?:\s+[^>]*)?>([\s\S]*?)<\/p>|<(div)([^>]*)>([\s\S]*?)<\/div>/gi;
    let last = 0;
    let m: RegExpExecArray | null;
    const state = { currentVerse: 0 };

    while ((m = re.exec(html))) {
      if (m.index > last) {
        const text = html.slice(last, m.index);
        if (text.trim()) {
          blocks.push(
            <p key={`blk-${idx++}`}>
              {renderInlineWithTooltips(text, opts, state, `inline-${idx}`)}
            </p>
          );
        }
      }
      if (m[1] === 'h3') {
        const attrs = m[2] || '';
        const inner = m[3] || '';
        const plain = inner.replace(/<\/?span[^>]*>/g, '');
        const kindMatch = attrs.match(/data-kind=\"([^\"]*)\"/);
        const kind = kindMatch ? kindMatch[1] : undefined;
        const order = kind === 'section' ? 2 : 3;

        // Process heading text to handle [cr:X] markers
        const headingContent = renderInlineWithTooltips(plain, opts, state, `h3-${idx}`);

        blocks.push(
          <Title key={`blk-${idx++}`} order={order} mb={kind === 'section' ? 8 : 0}>
            {headingContent}
          </Title>
        );
      } else if (m[4] === 'p') {
        const inner = m[5] || '';
        const plain = inner.replace(/<\/?span[^>]*>/g, '');
        blocks.push(
          <p key={`blk-${idx++}`} style={opts.isThomas ? { marginBottom: '1rem' } : undefined}>
            {renderInlineWithTooltips(plain, opts, state, `p-${idx}`)}
          </p>
        );
      } else if (m[6] === 'div') {
        const attrs = m[7] || '';
        const inner = m[8] || '';
        const plain = inner.replace(/<\/?span[^>]*>/g, '');
        const classMatch = attrs.match(/class=\"([^\"]*)\"/);
        const className = classMatch ? classMatch[1] : undefined;
        const styleMatch = attrs.match(/style=\"([^\"]*)\"/);
        const indentStyle = styleMatch ? styleMatch[1] : undefined;
        const styleObj = indentStyle
          ? Object.fromEntries(
              indentStyle
                .split(';')
                .filter(Boolean)
                .map((s) => {
                  const [k, v] = s.split(':');
                  return [
                    k.trim().replace(/-([a-z])/g, (_: any, c: string) => c.toUpperCase()),
                    v.trim(),
                  ];
                })
            )
          : undefined;
        blocks.push(
          <div key={`blk-${idx++}`} className={className} style={styleObj as any}>
            <span>{renderInlineWithTooltips(plain, opts, state, `div-${idx}`)}</span>
          </div>
        );
      }
      last = m.index + m[0].length;
    }

    if (last < html.length) {
      const text = html.slice(last);
      if (text.trim()) {
        blocks.push(
          <p key={`blk-${idx++}`}>{renderInlineWithTooltips(text, opts, state, `tail-${idx}`)}</p>
        );
      }
    }

    return <>{blocks}</>;
  }

  useEffect(() => {
    if (!sectionRef.current) return;

    let scrollContainer: HTMLElement | null = sectionRef.current
      .parentElement as HTMLElement | null;
    scrollContainer = findScrollViewport(scrollContainer);
    if (!scrollContainer) return;

    let paneReg = __paneRegMap.get(paneId);
    if (!paneReg) {
      paneReg = {
        container: scrollContainer,
        items: new Map(),
        handlerAttached: false,
        rafId: null,
        lastIndex: null,
        cleanupTimer: null,
      };
      __paneRegMap.set(paneId, paneReg);
    } else if (paneReg.container !== scrollContainer) {
      if (paneReg.handlerAttached) {
        paneReg.container.removeEventListener('scroll', onScroll as any, true);
        paneReg.handlerAttached = false;
      }
      paneReg.container = scrollContainer;
    }

    let lastScrollTop = 0;
    const SCROLL_DOWN_OFFSET = 200;
    const SCROLL_UP_OFFSET = 0;

    const computeAndActivate = () => {
      const reg = __paneRegMap.get(paneId);
      if (!reg || reg.items.size === 0) return;

      const scrollingUp = reg.container.scrollTop < lastScrollTop;
      lastScrollTop = reg.container.scrollTop;

      const TOP_OFFSET = scrollingUp ? SCROLL_UP_OFFSET : SCROLL_DOWN_OFFSET;

      const rootTop = reg.container.getBoundingClientRect().top;
      let bestIdx: number | null = null;
      let bestTop = -Infinity;

      for (const [i, el] of Array.from(reg.items.entries()).sort((a, b) => a[0] - b[0])) {
        const t = el.getBoundingClientRect().top;

        if (t <= rootTop + TOP_OFFSET && t > bestTop) {
          bestTop = t;
          bestIdx = i;
        }
      }

      if (bestIdx == null) bestIdx = 0;

      if (reg.lastIndex !== bestIdx) {
        reg.lastIndex = bestIdx!;
        updateActiveIndex(paneId, tabId, bestIdx!);
      }
    };

    function onScroll() {
      const reg = __paneRegMap.get(paneId);
      if (!reg) return;
      if (reg.rafId != null) return;
      reg.rafId = window.requestAnimationFrame(() => {
        const r = __paneRegMap.get(paneId);
        if (r) r.rafId = null;
        computeAndActivate();
      });
    }

    paneReg.items.set(index, sectionRef.current);

    if (!paneReg.handlerAttached) {
      paneReg.container.addEventListener('scroll', onScroll as any, {
        passive: true,
        capture: true,
      });
      paneReg.handlerAttached = true;
    }

    requestAnimationFrame(() => computeAndActivate());

    return () => {
      const reg = __paneRegMap.get(paneId);
      if (!reg) return;

      reg.items.delete(index);

      if (reg.items.size === 0) {
        if (reg.cleanupTimer) clearTimeout(reg.cleanupTimer);
        reg.cleanupTimer = window.setTimeout(() => {
          const r = __paneRegMap.get(paneId);
          if (!r) return;
          if (r.items.size === 0) {
            if (r.handlerAttached) {
              r.container.removeEventListener('scroll', onScroll as any, true);
              r.handlerAttached = false;
            }
            __paneRegMap.delete(paneId);
          }
        }, 0);
      }
    };
  }, [paneId, tabId, index]);

  if (!result) return null;
  if (!activeTab) return null;

  const fnTips = buildFootnoteTooltipMap(activeTab);
  const crTips = buildCrossrefTooltipMap(activeTab);
  const baseChapter = result.reference.chapter;

  const bookName = getFullBookName(result.reference.bookCode, booksGrouped);
  const chapter = result.reference.chapter;
  const verseStart = result.reference.verseStart;
  const verseEnd = result.reference.verseEnd;

  let referenceLabel = `${bookName} ${chapter}`;
  if (verseStart > 0) {
    referenceLabel += `:${verseStart}`;
    if (verseEnd > 0 && verseEnd !== verseStart) referenceLabel += `-${verseEnd}`;
  }

  return (
    <>
      {!result.isWholeChapter && (
        <Box
          mb="xs"
          py="xs"
          px="md"
          style={{
            borderTop: '1px solid #e0e0e0',
            borderBottom: '1px solid #e0e0e0',
            backgroundColor: '#f8f9fa',
          }}
        >
          <Text size="sm" c="dimmed" fw={500}>
            {referenceLabel}
          </Text>
        </Box>
      )}

      <Box
        ref={sectionRef}
        mb="xl"
        p="xs"
        data-section-index={index}
        data-pane-id={paneId}
        data-book-code={result.reference.bookCode}
        data-chapter={result.reference.chapter}
        data-verse-start={result.reference.verseStart}
        data-verse-end={result.reference.verseEnd}
      >
        {renderPassageReact(html, {
          show_fn: activeTab.footerOpen && activeTab.footerTab === 'footnotes',
          show_cr: activeTab.footerOpen && activeTab.footerTab === 'crossrefs',
          baseChapter,
          isThomas: result.reference.bookCode === 'Thomas',
          fnTips,
          crTips,
        })}
      </Box>
    </>
  );
}
