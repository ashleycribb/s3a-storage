const fs = require('fs');
const data = JSON.parse(fs.readFileSync('tests/adl-conformance/logs/38f32817-ee1a-43d9-a963-eb3951a3f7a1.log', 'utf8'));

function findFailures(item, acc = []) {
  if (!item) return acc;
  if (item.status === 'failed' || item.state === 'failed') {
    if (item.tests && item.tests.length > 0) {
      // suite failed, recurse
    } else {
      acc.push({
        title: item.title || item.name,
        error: item.error ? (item.error.message || item.error) : item.message,
        expected: item.expected,
        actual: item.actual
      });
    }
  }
  if (item.tests && Array.isArray(item.tests)) {
    for (const sub of item.tests) {
      findFailures(sub, acc);
    }
  }
  if (item.log) {
    if (Array.isArray(item.log)) {
      for (const sub of item.log) findFailures(sub, acc);
    } else if (typeof item.log === 'object') {
      findFailures(item.log, acc);
    }
  }
  return acc;
}

const failures = findFailures(data);
console.log('Total failed tests found:', failures.length);
const summary = {};
for (const f of failures) {
  const errPrefix = (String(f.error || 'Unknown error')).split('\n')[0].substring(0, 100);
  summary[errPrefix] = (summary[errPrefix] || 0) + 1;
}
console.log('Failure types:');
console.log(JSON.stringify(summary, null, 2));

console.log('\nAll failed tests:');
failures.forEach((f, i) => {
  console.log(`${i+1}: [${f.title}] -> ${f.error ? String(f.error).split('\n')[0] : 'no error'}`);
});
