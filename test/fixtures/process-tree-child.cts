const {spawn} = require('node:child_process');
const descendant = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {windowsHide:true, stdio:'ignore'});
descendant.once('spawn', () => process.stdout.write(`${descendant.pid}\n`));
setInterval(() => {}, 1000);
