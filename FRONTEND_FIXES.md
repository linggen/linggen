# Frontend Fixes - State Persistence & Progress Tracking

## ✅ Fixed Issues

### 1. **Fetch State on Startup**

**Problem:** When refreshing the page, all indexing jobs were lost.

**Solution:** Added `useEffect` to fetch jobs from backend on startup:

```typescript
useEffect(() => {
  const fetchJobs = async () => {
    const response = await listJobs();
    const backendJobs = response.jobs.map(/* convert to frontend format */);
    setJobs(backendJobs);

    // Restore indexing state if a job is still running
    const hasRunningJob = backendJobs.some((job) => job.status === "running");
    if (hasRunningJob) {
      setStatus("indexing");
      setIndexingResourceId(runningJob.sourceId);
      setIndexingProgress("Indexing in progress...");
    }
  };
  fetchJobs();
}, []);
```

**Result:**

- ✅ Jobs persist across page refreshes
- ✅ Running jobs are restored with correct state
- ✅ Activity view shows all historical jobs

---

### 2. **Real-Time Progress Updates**

**Problem:** Progress was fake (simulated with `setInterval`), didn't reflect actual backend progress.

**Solution:**

1. Use new `indexSource` API (instead of `indexFolder`)
2. Poll backend every second for job status
3. Display real progress from backend

```typescript
const handleIndexResource = async (resource: Resource) => {
  // Start indexing
  const result = await indexSource(resource.id);
  const jobId = result.job_id;

  // Poll for progress every second
  const pollInterval = setInterval(async () => {
    const response = await listJobs();
    const job = response.jobs.find((j) => j.id === jobId);

    if (job?.status === "Running") {
      // Show real progress!
      setIndexingProgress(
        `Processing... ${job.files_indexed} files, ${job.chunks_created} chunks`
      );
    } else if (job?.status === "Completed") {
      clearInterval(pollInterval);
      setIndexingProgress(`✓ Indexed ${job.files_indexed} files`);
      // ...
    }
  }, 1000);
};
```

**Result:**

- ✅ Shows actual file count and chunk count
- ✅ Updates every second with real backend data
- ✅ Displays completion status with final counts
- ✅ Shows errors if indexing fails

---

## 📊 What You'll See Now

### On Page Load:

```
1. Frontend calls /api/jobs
2. Backend returns all jobs from redb
3. Frontend displays job history in Activity view
4. If a job is running, shows "Indexing in progress..."
```

### During Indexing:

```
Starting...                           (0s)
Reading files...                      (1s)
Processing... 10 files, 85 chunks    (2s)
Processing... 23 files, 198 chunks   (3s)
Processing... 47 files, 412 chunks   (4s)
...
✓ Indexed 1000 files, 8542 chunks    (done)
```

### Backend Updates Progress:

```
Every 10 files:
  - job.files_indexed updated in redb
  - job.chunks_created updated in redb

Frontend polls every 1 second:
  - Fetches latest job status
  - Updates UI with real numbers
```

---

## 🔄 Data Flow

```
┌─────────────────────────────────────────────────────────┐
│  Frontend                                               │
│  ┌───────────────────────────────────────────────────┐ │
│  │  1. User clicks "Index now"                       │ │
│  │  2. POST /api/index_source { source_id: "..." }  │ │
│  │  3. Receives { job_id: "..." }                    │ │
│  │  4. Start polling every 1 second                  │ │
│  └───────────────────────────────────────────────────┘ │
│                        ↓                                │
│  ┌───────────────────────────────────────────────────┐ │
│  │  Poll Loop (every 1s):                            │ │
│  │  - GET /api/jobs                                  │ │
│  │  - Find job by job_id                             │ │
│  │  - Update UI with job.files_indexed, etc.         │ │
│  │  - Stop when status = Completed/Failed            │ │
│  └───────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
                         ↕
┌─────────────────────────────────────────────────────────┐
│  Backend                                                │
│  ┌───────────────────────────────────────────────────┐ │
│  │  POST /api/index_source                           │ │
│  │  1. Create job in redb (status: Running)          │ │
│  │  2. Start processing files                        │ │
│  │  3. Every 10 files: update job in redb            │ │
│  │  4. On completion: update job (status: Completed) │ │
│  └───────────────────────────────────────────────────┘ │
│                        ↕                                │
│  ┌───────────────────────────────────────────────────┐ │
│  │  GET /api/jobs                                    │ │
│  │  - Read all jobs from redb                        │ │
│  │  - Return as JSON                                 │ │
│  └───────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
                         ↕
┌─────────────────────────────────────────────────────────┐
│  redb (Persistent Storage)                              │
│  ┌───────────────────────────────────────────────────┐ │
│  │  JOBS_TABLE:                                      │ │
│  │  {                                                │ │
│  │    "job-123": {                                   │ │
│  │      "status": "Running",                         │ │
│  │      "files_indexed": 47,                         │ │
│  │      "chunks_created": 412,                       │ │
│  │      ...                                          │ │
│  │    }                                              │ │
│  │  }                                                │ │
│  └───────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

---

## 🧪 How to Test

### Test 1: State Persistence

1. Start backend: `cd backend/api && cargo run`
2. Open frontend: `http://localhost:3000`
3. Add a local folder source
4. Click "Index now"
5. **While indexing, refresh the page** (Ctrl+R / Cmd+R)
6. ✅ Should see: Job still running, progress restored

### Test 2: Real Progress

1. Index a folder with many files (100+)
2. Watch the progress text
3. ✅ Should see: Real file counts updating every second
4. ✅ Should see: "Processing... 23 files, 198 chunks" (not fake progress)

### Test 3: Job History

1. Index several folders
2. Go to Activity view
3. ✅ Should see: All completed jobs with file/chunk counts
4. Refresh page
5. ✅ Should see: Jobs still there (persisted in redb)

---

## 📝 Files Modified

### Frontend:

- **`frontend/src/App.tsx`**
  - Added `useEffect` to fetch jobs on startup
  - Updated `handleIndexResource` to use `indexSource` API
  - Added polling logic for real-time progress
  - Removed fake progress simulation

### Backend (No Changes Needed):

- Already has `/api/index_source` endpoint
- Already has `/api/jobs` endpoint
- Already updates job progress in redb every 10 files

---

## 🚀 Next Steps (Optional)

### 1. WebSocket for Real-Time Updates (Better than polling)

Instead of polling every second, use WebSocket:

```typescript
const ws = new WebSocket("ws://localhost:3000/ws");
ws.onmessage = (event) => {
  const job = JSON.parse(event.data);
  updateJobProgress(job);
};
```

### 2. Progress Bar

Show visual progress bar:

```typescript
const progress = (job.files_indexed / totalFiles) * 100
<div className="progress-bar" style={{ width: `${progress}%` }} />
```

### 3. Cancel Indexing

Add ability to cancel running jobs:

```typescript
<button onClick={() => cancelJob(jobId)}>Cancel</button>
```

---

## ✨ Summary

**Before:**

- ❌ Jobs lost on page refresh
- ❌ Fake progress (simulated)
- ❌ No real-time updates

**After:**

- ✅ Jobs persist in redb
- ✅ Real progress from backend
- ✅ Updates every second
- ✅ Shows actual file/chunk counts
- ✅ Restores state on page load

🎉 **Frontend now fully integrated with backend job tracking!**
