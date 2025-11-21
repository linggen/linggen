# How to See Progress on Webpage

## 🔴 Problem

The backend logs show it's using the **old `index_folder` handler** which doesn't update job progress in redb. That's why the frontend can't see progress!

From your logs:

```
INFO api/src/handlers/index_folder.rs:45: Starting ingestion for folder: ...
```

Should be:

```
INFO api/src/handlers/index_source.rs:81: Starting ingestion for folder: ...
```

## ✅ Solution: Restart Backend

### Step 1: Stop Current Backend

In your backend terminal, press **Ctrl+C** to stop the server.

### Step 2: Restart Backend

```bash
cd /Users/lianghuang/workspace/rust/rememberme/backend/api
cargo run
```

### Step 3: Refresh Frontend

In your browser, **hard refresh** the page:

- **Mac:** Cmd+Shift+R
- **Windows/Linux:** Ctrl+Shift+R

### Step 4: Test

1. Go to **Sources** view
2. Click **"Index now"** on your "sailsame" resource
3. Watch the progress text - you should now see:
   ```
   Starting...
   Reading files...
   Processing... 10 files, 87 chunks
   Processing... 23 files, 201 chunks
   ...
   ```

---

## 🔍 How to Verify It's Working

### Backend Logs Should Show:

```
INFO api/src/handlers/index_source.rs:58: Started job <job-id> for source 'sailsame'
INFO api/src/handlers/index_source.rs:81: Starting ingestion for folder: ...
INFO api/src/handlers/index_source.rs:123: [0.1%] Processing 1/1287: ...
INFO api/src/handlers/index_source.rs:193:   💾 Writing batch of X chunks to LanceDB...
INFO api/src/handlers/index_source.rs:205:   ✓ Batch written in X.XXms
```

**Key indicator:** Should say `index_source.rs`, NOT `index_folder.rs`!

### Frontend Should Show:

- **Status badge:** "🔵 Indexing" (top right)
- **Progress text:** Real file/chunk counts updating every second
- **Index button:** Shows "⏳ Processing... X files, Y chunks" with spinner

---

## 🐛 If Still Not Working

### Check 1: Backend Using New Code?

Look at backend logs when you click "Index now". If you see:

```
api/src/handlers/index_folder.rs:45
```

Then backend is still using old code. **Restart backend again.**

### Check 2: Frontend Using New Code?

Open browser DevTools (F12) → Network tab:

- Should see `POST /api/index_source` (NOT `/api/index_folder`)
- Should see `GET /api/jobs` every second

If you see `/api/index_folder`, then:

1. Clear browser cache
2. Hard refresh (Cmd+Shift+R / Ctrl+Shift+R)

### Check 3: Job Being Created?

After clicking "Index now", check backend logs for:

```
INFO api/src/handlers/index_source.rs:58: Started job <uuid> for source 'sailsame'
```

If you don't see this, the frontend is still using old code.

---

## 📝 Why This Happened

The backend serves static files from `backend/api/dist/`. When you:

1. Make frontend changes
2. Build frontend (`npm run build`)
3. Copy to `backend/api/dist/`

The backend needs to be **restarted** to serve the new files (or you need to hard-refresh the browser to bypass cache).

---

## ✨ Expected Behavior After Restart

1. **Click "Index now"**
2. **Backend logs:**

   ```
   INFO api/src/handlers/index_source.rs:58: Started job abc-123 for source 'sailsame'
   INFO api/src/handlers/index_source.rs:123: [0.1%] Processing 1/1287: ...
   ```

3. **Frontend shows:**

   ```
   Starting...                          (immediately)
   Reading files...                     (1 second)
   Processing... 1 files, 1 chunks     (2 seconds)
   Processing... 3 files, 9 chunks     (3 seconds)
   Processing... 6 files, 18 chunks    (4 seconds)
   ...
   ```

4. **Updates every second** with real numbers from backend!

---

## 🎯 Quick Checklist

- [ ] Stop backend (Ctrl+C)
- [ ] Start backend (`cargo run`)
- [ ] Hard refresh browser (Cmd+Shift+R)
- [ ] Click "Index now"
- [ ] Check backend logs say `index_source.rs`
- [ ] Check frontend shows real progress numbers

If all checked, progress should work! 🎉
