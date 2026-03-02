import React, { useMemo, useState, useEffect } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { LanguageDescription } from '@codemirror/language';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView } from '@codemirror/view';
import type { Extension } from '@codemirror/state';
import {
  livePreviewPlugin,
  livePreviewTheme,
  livePreviewLightTheme,
} from './cm6-live-preview';

export const CM6Editor: React.FC<{
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  livePreview?: boolean;
  /** File path used for syntax detection. Falls back to markdown when absent. */
  filePath?: string;
}> = ({ value, onChange, readOnly = false, livePreview = false, filePath }) => {
  const isDark = useMemo(() => {
    if (typeof window === 'undefined') return false;
    return document.documentElement.classList.contains('dark');
  }, []);

  // Resolve language extension from filePath via @codemirror/language-data
  const [langExt, setLangExt] = useState<Extension | null>(null);
  const isMarkdownFile = !filePath || /\.md$/i.test(filePath);

  useEffect(() => {
    if (!filePath || isMarkdownFile) {
      setLangExt(null);
      return;
    }
    const desc = LanguageDescription.matchFilename(languages, filePath);
    if (desc) {
      desc.load().then((support) => setLangExt(support));
    } else {
      setLangExt(null);
    }
  }, [filePath, isMarkdownFile]);

  const extensions = useMemo(() => {
    const exts: Extension[] = [];
    if (isMarkdownFile) {
      exts.push(
        markdown({
          base: markdownLanguage,
          codeLanguages: languages,
        }),
      );
    } else if (langExt) {
      exts.push(langExt);
    }
    exts.push(EditorView.lineWrapping);
    if (livePreview && isMarkdownFile) {
      exts.push(livePreviewPlugin);
      exts.push(livePreviewTheme);
      if (!isDark) {
        exts.push(livePreviewLightTheme);
      }
    }
    return exts;
  }, [livePreview, isDark, isMarkdownFile, langExt]);

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      readOnly={readOnly}
      height="auto"
      theme={isDark ? oneDark : 'light'}
      extensions={extensions}
      basicSetup={{
        lineNumbers: !livePreview,
        foldGutter: true,
        highlightActiveLineGutter: !livePreview,
        highlightActiveLine: true,
        history: true,
      }}
    />
  );
};
