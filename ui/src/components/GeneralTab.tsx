import React from 'react';
import type { AppConfig } from '../types';
import { WeatherSettings } from './WeatherSettings';
import { serverAddr, withServerAddr } from '../lib/serverAddr';
import { Field, Grid, MoreGroup, Section, inputCls, sectionCls } from './settings/kit';

/** Settings → General: language, the sky, and — folded away — the server
 *  and logging knobs. Agents, Session and Memory have their own tabs. */
export const GeneralTab: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => (
  <div className="space-y-6">
    <Section title="Language">
      <Grid>
        <Field hint="Pages, reports and missions use this. Chat replies follow what you write.">
          <select
            className={inputCls}
            value={config.language ?? ''}
            onChange={(e) => onChange({ ...config, language: e.target.value || null })}
          >
            <option value="">Auto (follows the Mac)</option>
            <option value="en">English</option>
            <option value="zh-Hans">中文</option>
          </select>
        </Field>
      </Grid>
    </Section>

    <WeatherSettings className={sectionCls} />

    <Section title="Advanced" desc="Server and logs. Rarely changed.">
      <MoreGroup label="Server">
        <Grid>
          <Field
            label="Port"
            hint="Where Linggen listens. Takes effect after a restart."
            details="Opening Linggen to your Wi-Fi is Phone access, on the Phone tab."
          >
            <input
              className={inputCls}
              type="number"
              min={1}
              max={65535}
              value={serverAddr(config.server).port}
              onChange={(e) => {
                const port = parseInt(e.target.value, 10);
                if (port >= 1 && port <= 65535) onChange({ ...config, server: withServerAddr(config.server, { port }) });
              }}
            />
          </Field>
        </Grid>
      </MoreGroup>
      <MoreGroup label="Logging">
        <Grid>
          <Field label="Level" hint="How much the engine writes to its log.">
            <select
              className={inputCls}
              value={config.logging.level || ''}
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, level: e.target.value || null } })}
            >
              <option value="">Default</option>
              <option value="trace">trace</option>
              <option value="debug">debug</option>
              <option value="info">info</option>
              <option value="warn">warn</option>
              <option value="error">error</option>
            </select>
          </Field>
          <Field label="Retention days" hint="Older logs are deleted. Blank = default.">
            <input
              className={inputCls}
              type="number"
              min={1}
              value={config.logging.retention_days ?? ''}
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, retention_days: e.target.value ? parseInt(e.target.value) : null } })}
              placeholder="(default)"
            />
          </Field>
          <Field label="Directory" hint="Where logs are written. Blank = default." wide>
            <input
              className={inputCls}
              value={config.logging.directory || ''}
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, directory: e.target.value || null } })}
              placeholder="(default)"
            />
          </Field>
        </Grid>
      </MoreGroup>
    </Section>
  </div>
);
