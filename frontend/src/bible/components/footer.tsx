import { IconX } from '@tabler/icons-react';
import { ActionIcon, Anchor, Box, Flex, ScrollArea, Tabs, Text } from '@mantine/core';
import { usePage } from '../pageContext';
import { usePanes } from '../paneContext';
import type { FootnoteItem, PaneTab, PassageResult } from '../types';
import type { JSX } from 'react';

interface FooterProps {
  paneId: number;
  activeTab: PaneTab;
  activeResult: PassageResult;
}

export function Footer({ paneId, activeTab, activeResult }: FooterProps) {
  const { booksGrouped, getFullBookName } = usePage();
  const { toggleFooter, closeFooter, handleReferenceClick } = usePanes();

  function parseCrossRefId(
    refId: string
  ): { label?: string; baseBookCode: string; baseChapter: number; baseVerse: number } | null {
    const m = refId.match(/^\[?([1-3]?[A-Za-z]+):(\d+):(\d+)(?:-([A-Za-z]+))?\]?$/);
    if (!m) return null;
    return {
      baseBookCode: m[1],
      baseChapter: parseInt(m[2], 10),
      baseVerse: parseInt(m[3], 10),
      label: m[4],
    };
  }

  function renderFootnoteHtml(fn: FootnoteItem, referenceBookCode: string): JSX.Element {
    const parsed = parseFootnoteId(fn.noteId);
    const baseBookName = parsed
      ? getFullBookName(parsed.baseBookCode, booksGrouped)
      : getFullBookName(referenceBookCode, booksGrouped);
    const baseChapter = parsed ? parsed.baseChapter : fn.chapter;
    const baseVerse = parsed ? parsed.baseVerse : fn.verse;

    let marker = fn.marker ?? parsed?.label?.toLowerCase() ?? '';
    if (!marker) {
      const m = String(fn.noteId ?? '').match(/-(\w+)$/);
      if (m) marker = m[1].toLowerCase();
    }
    if (!marker && /^[A-Za-z]$/.test(String(fn.noteId ?? ''))) {
      marker = String(fn.noteId).toLowerCase();
    }
    const label = marker ? `${marker}. ` : '';

    let text = String((fn as any).text ?? '');
    text = text.replace(/\{\{REF(\d+)\}\}/g, (_m, g1) => {
      const html = buildRefHtmlFromCollection((fn as any).references, Number(g1));
      return html ? html : '';
    });

    text = convertMarkdownTablesToHtml(text);

    const baseRef = `${baseBookName} ${baseChapter}:${baseVerse}`;
    const baseRefString = `${baseBookName} ${baseChapter}:${baseVerse}`;

    return (
      <>
        {label}
        <Anchor
          component="button"
          onClick={(e) => {
            e.preventDefault();
            handleReferenceClick(paneId, baseRefString);
          }}
          size="sm"
          style={{ cursor: 'pointer' }}
        >
          {baseRef}
        </Anchor>
        {' - '}
        <span dangerouslySetInnerHTML={{ __html: text }} />
      </>
    );
  }

  function buildRefHtmlFromCollection(refs: any, keyOrIndex: string | number): string {
    let items: any[] | null = null;
    if (Array.isArray(refs)) {
      const idx =
        typeof keyOrIndex === 'number'
          ? keyOrIndex
          : parseInt(String(keyOrIndex).replace(/\D/g, ''), 10);
      if (!isNaN(idx) && refs[idx - 1]) {
        const sel = refs[idx - 1];
        items = Array.isArray(sel) ? sel : [sel];
      }
    } else if (refs && typeof refs === 'object') {
      const sel = (refs as any)[String(keyOrIndex)] ?? (refs as any)[`REF${keyOrIndex}`];
      if (sel) items = Array.isArray(sel) ? sel : [sel];
    }
    if (!items || items.length === 0) return '';
    return items.map(formatRefItem).join('; ');
  }

  function parseFootnoteId(
    noteId: string
  ): { label?: string; baseBookCode: string; baseChapter: number; baseVerse: number } | null {
    const m = noteId.match(/^\[?([1-3]?[A-Za-z]+):(\d+):(\d+)(?:-([A-Za-z]+))?\]?$/);
    if (!m) return null;
    return {
      baseBookCode: m[1],
      baseChapter: parseInt(m[2], 10),
      baseVerse: parseInt(m[3], 10),
      label: m[4],
    };
  }

  function formatRefItem(ri: any): string {
    if (!ri) return '';
    const bookName = typeof ri.book === 'string' ? ri.book : String(ri.book ?? '');
    const chap = ri.chapter ?? '';
    const verse = ri.verse ?? '';
    const bookFull = getFullBookName(bookName, booksGrouped);
    if (chap && verse) return `${bookFull} ${chap}:${verse}`;
    if (chap) return `${bookFull} ${chap}`;
    return bookFull;
  }

  function convertMarkdownTablesToHtml(text: string): string {
    const lines = text.split(/\r?\n/);
    const out: string[] = [];
    let i = 0;
    while (i < lines.length) {
      if (/^\s*\|.*\|\s*$/.test(lines[i])) {
        const tableLines: string[] = [];
        while (i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i])) {
          tableLines.push(lines[i]);
          i++;
        }
        if (tableLines.length >= 2 && /\|\s*-{3,}\s*\|/.test(tableLines[1])) {
          const parseRow = (row: string) =>
            row
              .trim()
              .slice(1, -1)
              .split('|')
              .map((c) => c.trim());
          const rows: string[] = [];
          rows.push(...parseRow(tableLines[0]));
          const bodyRows = tableLines.slice(2).map(parseRow);
          const allRows = [parseRow(tableLines[0]), ...bodyRows];
          out.push(
            '<table><tbody>' +
              allRows
                .map((r) => '<tr>' + r.map((c) => `<td>${c}</td>`).join('') + '</tr>')
                .join('') +
              '</tbody></table>'
          );
          continue;
        }
        out.push(tableLines.join(' '));
        continue;
      }
      out.push(lines[i]);
      i++;
    }
    return out.join('\n');
  }

  return (
    <Box style={{ borderTop: '1px solid #dee2e6' }}>
      <Tabs
        value={activeTab.footerOpen ? activeTab.footerTab : null}
        onChange={(value) => toggleFooter(paneId, activeTab.id, value as 'footnotes' | 'crossrefs')}
        styles={
          activeTab.footerOpen
            ? {}
            : {
                root: {
                  '--tab-border-color': 'transparent',
                },
              }
        }
      >
        <Flex justify="space-between" align="center">
          <Tabs.List>
            <Tabs.Tab value="footnotes">Footnotes</Tabs.Tab>
            <Tabs.Tab value="crossrefs">Cross References</Tabs.Tab>
          </Tabs.List>
          {activeTab.footerOpen && (
            <ActionIcon variant="subtle" onClick={() => closeFooter(paneId, activeTab.id)} mr="xs">
              <IconX size={16} />
            </ActionIcon>
          )}
        </Flex>

        {activeTab.footerOpen && (
          <Box style={{ height: '300px', overflow: 'hidden' }}>
            <Tabs.Panel value="footnotes" h="100%">
              <ScrollArea h="100%" scrollbars="y" type="auto" offsetScrollbars>
                <Box p="xs">
                  {activeResult.footnotes && activeResult.footnotes.length > 0 ? (
                    activeResult.footnotes.map((fn) => (
                      <Box key={`fn-${fn.chapter}-${fn.verse}-${fn.noteId}`} mb="sm">
                        <Text id={`fn-${fn.chapter}-${fn.verse}-${fn.noteId}`} size="sm">
                          {renderFootnoteHtml(fn as any, activeResult.reference.bookCode)}
                        </Text>
                      </Box>
                    ))
                  ) : (
                    <Text size="sm" c="dimmed">
                      No footnotes for this passage.
                    </Text>
                  )}
                </Box>
              </ScrollArea>
            </Tabs.Panel>

            <Tabs.Panel value="crossrefs" h="100%">
              <ScrollArea h="100%" scrollbars="y" type="auto" offsetScrollbars>
                <Box p="xs">
                  {activeResult.crossrefs && activeResult.crossrefs.length > 0 ? (
                    activeResult.crossrefs.map((cr) => {
                      let parsed = parseCrossRefId(cr.refId);
                      let baseBookCode: string;
                      let baseChapter: number;
                      let baseVerse: number;
                      let label: string | undefined = parsed?.label;
                      if (!parsed) {
                        const m = String(cr.refId).match(/^(\d+):(\d+)-([A-Za-z]+)$/);
                        if (m) {
                          baseBookCode = activeResult.reference.bookCode;
                          baseChapter = parseInt(m[1], 10);
                          baseVerse = parseInt(m[2], 10);
                          label = m[3];
                        } else {
                          return (
                            <Box key={`cr-${cr.chapter}-${cr.verse}-${cr.refId}`} mb="sm">
                              <Text size="sm">{cr.refId}</Text>
                            </Box>
                          );
                        }
                      } else {
                        baseBookCode = parsed.baseBookCode;
                        baseChapter = parsed.baseChapter;
                        baseVerse = parsed.baseVerse;
                      }
                      const baseBookName = getFullBookName(baseBookCode, booksGrouped);
                      const baseRef = `${baseBookName} ${baseChapter}:${baseVerse}`;
                      const links = cr.links;
                      const formatLink = (l: any) => {
                        const name = getFullBookName(l.book, booksGrouped);
                        if (l?.verseStart !== undefined && l?.verseEnd !== undefined) {
                          return `${name} ${l.chapter}:${l.verseStart}-${l.verseEnd}`;
                        }
                        if (l?.verse !== undefined) {
                          return `${name} ${l.chapter}:${l.verse}`;
                        }
                        if (l?.chapter !== undefined) {
                          return `${name} ${l.chapter}`;
                        }
                        return name;
                      };

                      let linksArray: any[] = [];
                      if (Array.isArray(links)) {
                        linksArray = links;
                      } else if (typeof links === 'string') {
                        try {
                          const arr = JSON.parse(links);
                          if (Array.isArray(arr)) {
                            linksArray = arr;
                          }
                        } catch {}
                      }

                      const allLinksText = linksArray.map((l: any) => formatLink(l)).join('; ');

                      return (
                        <Box key={`cr-${cr.chapter}-${cr.verse}-${cr.refId}`} mb="sm">
                          <Text size="sm" component="div">
                            {label ? `${label.toUpperCase()}. ` : ''}
                            <Anchor
                              component="button"
                              onClick={(e) => {
                                e.preventDefault();
                                handleReferenceClick(paneId, baseRef);
                              }}
                              size="sm"
                              style={{ cursor: 'pointer' }}
                            >
                              {baseRef}
                            </Anchor>
                            {linksArray.length > 0 && (
                              <>
                                {' : '}
                                <Anchor
                                  component="button"
                                  onClick={(e) => {
                                    e.preventDefault();
                                    handleReferenceClick(paneId, allLinksText);
                                  }}
                                  size="sm"
                                  style={{ cursor: 'pointer' }}
                                >
                                  {allLinksText}
                                </Anchor>
                              </>
                            )}
                          </Text>
                        </Box>
                      );
                    })
                  ) : (
                    <Text size="sm" c="dimmed">
                      No cross references for this passage.
                    </Text>
                  )}
                </Box>
              </ScrollArea>
            </Tabs.Panel>
          </Box>
        )}
      </Tabs>
    </Box>
  );
}
