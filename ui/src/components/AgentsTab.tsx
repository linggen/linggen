import React from 'react';
import { FilePlus2, FileText, Save, Trash2 } from 'lucide-react';
import { CM6Editor } from './CM6Editor';
import { useAgentFiles } from '../hooks/useAgentFiles';

export const AgentsTab: React.FC<{
  projectRoot: string;
  onChanged?: () => void;
}> = ({ projectRoot, onChanged }) => {
  const {
    files, selectedPath, content, setContent, loadingList, loadingFile, saving, validationError, dirty,
    selectFile, saveFile, createFile, deleteFile,
  } = useAgentFiles(projectRoot, true, onChanged);

  return (
    <div className="flex h-full min-h-0">
      {/* Sidebar: agent list */}
      <aside className="w-64 border-r border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-black/20 flex flex-col shrink-0">
        <div className="px-3 py-2 border-b border-slate-200 dark:border-white/10 flex items-center justify-between">
          <div className="text-xs font-bold uppercase tracking-wide text-slate-600 dark:text-slate-300">Agent Specs</div>
          <button className="p-1.5 rounded hover:bg-slate-200 dark:hover:bg-white/10" onClick={createFile} title="New agent file">
            <FilePlus2 size={14} />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-2 space-y-1">
          {loadingList ? (
            <div className="text-xs text-slate-500 p-2">Loading...</div>
          ) : files.length === 0 ? (
            <div className="text-xs text-slate-500 p-2">No agent markdown files found.</div>
          ) : (
            files.map((file) => {
              return (
                <button
                  key={file.path}
                  onClick={() => selectFile(file.path)}
                  className={`w-full text-left px-2 py-1.5 rounded border ${
                    selectedPath === file.path
                      ? 'border-blue-300 bg-blue-50 dark:bg-blue-500/10 text-blue-700 dark:text-blue-300'
                      : 'border-transparent hover:border-slate-200 dark:hover:border-white/10'
                  }`}
                >
                  <div className="flex items-center gap-1.5">
                    <span className="text-xs font-semibold truncate flex-1">{file.agent_id}</span>
                  </div>
                  <div className="text-[12px] text-slate-500 truncate">{file.path}</div>
                </button>
              );
            })
          )}
        </div>
      </aside>

      {/* Editor area */}
      <section className="flex-1 min-w-0 overflow-y-auto">
        {/* Toolbar */}
        <div className="sticky top-0 z-10 px-3 py-2 border-b border-slate-200 dark:border-white/10 flex items-center justify-between bg-white dark:bg-[#0a0a0a]">
          <div className="flex items-center gap-2 min-w-0">
            <FileText size={14} className="text-slate-500 shrink-0" />
            <span className="text-xs font-mono truncate">{selectedPath || 'No file selected'}</span>
          </div>
          <div className="flex items-center gap-1.5">
            {dirty && <span className="text-[12px] text-amber-600">Unsaved</span>}
            <button
              onClick={saveFile}
              disabled={!selectedPath || saving || loadingFile}
              className="px-2 py-1 rounded text-xs border border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5 disabled:opacity-50"
              title="Save"
            >
              <span className="inline-flex items-center gap-1"><Save size={12} /> Save</span>
            </button>
            <button
              onClick={deleteFile}
              disabled={!selectedPath || saving}
              className="px-2 py-1 rounded text-xs border border-red-200 text-red-600 hover:bg-red-50 disabled:opacity-50"
              title="Delete"
            >
              <span className="inline-flex items-center gap-1"><Trash2 size={12} /> Delete</span>
            </button>
          </div>
        </div>

        {validationError && (
          <div className="px-3 py-2 text-xs bg-red-50 text-red-700 border-b border-red-100">{validationError}</div>
        )}

        {/* Live preview editor */}
        <div>
          {loadingFile ? (
            <div className="h-full flex items-center justify-center text-xs text-slate-500">Loading file...</div>
          ) : (
            <CM6Editor value={content} onChange={setContent} livePreview />
          )}
        </div>
      </section>
    </div>
  );
};
