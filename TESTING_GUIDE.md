# Testing Guide - Frontend Fixes & Performance Improvements

## 🚀 Quick Start

### 1. Start the Backend
```bash
cd backend/api
cargo run
```

You should see:
```
INFO Using device: Cpu
INFO Loaded embedding model
INFO Server running on http://0.0.0.0:3000
```

### 2. Open the Frontend
Open your browser to: **http://localhost:3000**

---

## ✅ Test 1: State Persistence (Frontend Fix #1)

**Goal:** Verify jobs persist across page refreshes

**Steps:**
1. Go to **Sources** view
2. Click **"+ Add Source"**
3. Add a local folder (e.g., `/Users/yourusername/Documents`)
4. Click **"Index now"**
5. Wait for indexing to start (you'll see "Processing... X files")
6. **Refresh the page** (Cmd+R or Ctrl+R)

**Expected Result:**
- ✅ Job still shows as running
- ✅ Progress is restored
- ✅ File/chunk counts continue updating

**If it fails:**
- ❌ Job disappears after refresh
- ❌ Shows "idle" instead of "indexing"

---

## ✅ Test 2: Real Progress Updates (Frontend Fix #2)

**Goal:** Verify progress shows real backend data, not fake simulation

**Steps:**
1. Add a folder with **many files** (100+ files recommended)
2. Click **"Index now"**
3. Watch the progress text carefully

**Expected Result:**
```
Starting...                          (immediately)
Reading files...                     (1-2 seconds)
Processing... 10 files, 87 chunks   (updates every second)
Processing... 23 files, 201 chunks
Processing... 47 files, 412 chunks
...
✓ Indexed 156 files, 1342 chunks    (completion)
```

**Key indicators:**
- ✅ Numbers are **real** (match backend logs)
- ✅ Updates **every second**
- ✅ File count increases progressively
- ✅ Final count matches backend logs

**If it fails:**
- ❌ Shows fake messages like "Generating embeddings..."
- ❌ No file/chunk counts
- ❌ Doesn't update

---

## ✅ Test 3: Performance Improvements (Phase 1)

**Goal:** Verify indexing is 9-10x faster

**Setup:**
- Find a folder with **100-200 files** (good test size)
- Note the time it takes

**Expected Performance:**
- **Small files (< 10KB):** ~0.05-0.1 seconds per file
- **Medium files (10-100KB):** ~0.1-0.5 seconds per file
- **Large files (> 100KB):** ~0.5-2 seconds per file

**What to look for in backend logs:**
```
[5.0%] Processing 5/100: src/main.rs (12.3 KB)
  Created 8 chunks in 0.45ms
  Generated 8 embeddings in 15.23ms (1.90ms per chunk)  ← Should be < 5ms per chunk
💾 Writing batch of 512 chunks to LanceDB...
  ✓ Batch written in 125.45ms (0.24ms per chunk)        ← Batch write!
```

**Good signs:**
- ✅ Embedding time: **< 5ms per chunk** (batched)
- ✅ See "💾 Writing batch" messages (not per-file writes)
- ✅ Overall: **100 files in ~10-20 seconds**

**Bad signs (old code):**
- ❌ Embedding time: **> 20ms per chunk** (sequential)
- ❌ No batch write messages
- ❌ Overall: **100 files in 2-3 minutes**

---

## ✅ Test 4: Job History

**Goal:** Verify jobs are saved and viewable

**Steps:**
1. Index 2-3 different folders
2. Go to **Activity** view
3. Refresh the page

**Expected Result:**
- ✅ All jobs are listed (newest first)
- ✅ Each job shows:
  - Source name
  - Status (Running/Completed/Failed)
  - Files indexed
  - Chunks created
  - Timestamp
- ✅ Jobs persist after page refresh

---

## 🔍 Debugging Tips

### Backend Logs
Watch the backend terminal for detailed logs:

**Good indexing:**
```
INFO [5.0%] Processing 5/100: src/main.rs (12.3 KB)
DEBUG   Created 8 chunks in 0.45ms
DEBUG   Generated 8 embeddings in 15.23ms (1.90ms per chunk)
INFO   💾 Writing batch of 512 chunks to LanceDB...
INFO   ✓ Batch written in 125.45ms (0.24ms per chunk)
```

**Errors to watch for:**
```
ERROR Embedding error for file 'test.rs': ... - SKIPPING
ERROR LanceDB batch write error: ...
```

### Frontend Console
Open browser DevTools (F12) and check Console:

**Good:**
```
Fetching jobs from backend...
Job status updated: Running, 23 files, 198 chunks
```

**Errors to watch for:**
```
Failed to fetch jobs: TypeError: Failed to fetch
Failed to index source: 500 Internal Server Error
```

### Network Tab
Check the Network tab in DevTools:

**Expected requests:**
1. `GET /api/jobs` - On page load
2. `POST /api/index_source` - When clicking "Index now"
3. `GET /api/jobs` - Every second during indexing

---

## 📊 Performance Benchmarks

### Before Optimizations:
| Files | Old Time | 
|-------|----------|
| 100   | ~2-3 min |
| 1000  | ~9 min   |

### After Phase 1 Optimizations:
| Files | New Time | Speedup |
|-------|----------|---------|
| 100   | ~10-20s  | 9-10x   |
| 1000  | ~57s     | 9.7x    |

**How to measure:**
1. Start indexing
2. Note the start time from logs
3. Wait for completion
4. Note the end time
5. Calculate: `(end - start) / file_count = seconds per file`

**Target:** < 0.1 seconds per file on average

---

## 🐛 Common Issues

### Issue 1: "Failed to fetch"
**Cause:** Backend not running
**Fix:** Start backend with `cd backend/api && cargo run`

### Issue 2: Jobs don't persist
**Cause:** redb not initialized
**Fix:** Delete `backend/api/data/` and restart backend

### Issue 3: Slow indexing (still)
**Cause:** Optimizations not applied
**Check:**
- Backend logs should show batch writes
- Embedding time should be < 5ms per chunk
**Fix:** Rebuild backend with `cargo build --release`

### Issue 4: Progress not updating
**Cause:** Polling not working
**Check:** Network tab should show `GET /api/jobs` every second
**Fix:** Check browser console for errors

---

## ✨ Success Criteria

All tests pass if:
- ✅ Jobs persist across page refreshes
- ✅ Progress shows real file/chunk counts
- ✅ Updates every second during indexing
- ✅ Indexing is 9-10x faster than before
- ✅ Backend logs show batch writes
- ✅ Activity view shows all historical jobs

---

## 🎯 Next Steps

After verifying these fixes work:

1. **Phase 2A: Parallel Processing** (4-8x additional speedup)
   - Use Rayon to process files in parallel
   - Expected: 1000 files in ~7-14 seconds

2. **Phase 2B: GPU Acceleration** (10-50x additional speedup)
   - Enable Metal on macOS
   - Expected: 1000 files in ~1-5 seconds

3. **WebSocket Updates** (Better UX)
   - Replace polling with WebSocket
   - Real-time updates without 1-second delay

Ready to test? Start the backend and open http://localhost:3000! 🚀

