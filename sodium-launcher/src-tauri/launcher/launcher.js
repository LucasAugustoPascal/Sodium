const { Client } = require("minecraft-launcher-core");

const version = process.argv[2];
const access_token = process.argv[3];
const uuid = process.argv[4];
const name = process.argv[5];

const launcher = new Client();

const opts = {
    authorization: {
        access_token,
        uuid,
        name
    },
    root: "./mc-data",
    version: {
        number: version,
        type: "release"
    }
};

launcher.launch(opts);

launcher.on("debug", console.log);
launcher.on("data", console.log);