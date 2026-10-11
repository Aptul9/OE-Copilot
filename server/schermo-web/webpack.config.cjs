// Costruzione del servizio (dist/index.js) e della pagina (dist/public). Uso: webpack --config webpack.config.cjs
// Le voci di webpack/default.build.config.json e di build.config.override.json scelgono i blocchi "/// #if" del codice.
const fs = require('fs');
const path = require('path');
const webpack = require('webpack');
const nodeExternals = require('webpack-node-externals');
const MiniCssExtractPlugin = require('mini-css-extract-plugin');
const HtmlWebpackPlugin = require('html-webpack-plugin');
const GeneratePackageJsonPlugin = require('@dead50f7/generate-package-json-webpack-plugin');

const ROOT = __dirname;
const readJson = (file) => JSON.parse(fs.readFileSync(path.join(ROOT, file)).toString());
const buildConfig = { ...readJson('webpack/default.build.config.json'), ...readJson('build.config.override.json') };

const rules = () => [
    { test: /\.css$/i, use: [MiniCssExtractPlugin.loader, 'css-loader'] },
    { test: /\.tsx?$/, use: [{ loader: 'ts-loader' }, { loader: 'ifdef-loader', options: buildConfig }], exclude: /node_modules/ },
    { test: /\.svg$/, loader: 'svg-inline-loader' },
    { test: /\.(png|jpe?g|gif)$/i, use: [{ loader: 'file-loader' }] },
    { test: /\.jar$/, use: [{ loader: 'file-loader', options: { name: '[path][name].[ext]' } }] },
    { test: /LICENSE/i, use: [{ loader: 'file-loader', options: { name: '[path][name]' } }] },
];

const frontend = {
    name: 'pagina',
    mode: 'production',
    entry: path.join(ROOT, 'src/app/index.ts'),
    externals: ['fs'],
    module: { rules: rules() },
    plugins: [
        new HtmlWebpackPlugin({ template: path.join(ROOT, 'src/public/index.html'), inject: 'head' }),
        new MiniCssExtractPlugin(),
        new webpack.ProvidePlugin({ Buffer: ['buffer', 'Buffer'] }),
    ],
    resolve: { extensions: ['.tsx', '.ts', '.js'], fallback: { path: 'path-browserify' } },
    output: { filename: 'bundle.js', path: path.join(ROOT, 'dist/public') },
};

const { name, version, description, author, license, scripts } = readJson('package.json');
const backend = {
    name: 'servizio',
    mode: 'production',
    entry: path.join(ROOT, 'src/server/index.ts'),
    externals: [nodeExternals()],
    module: { rules: rules() },
    plugins: [
        // package.json del pacchetto: solo le dipendenze che il servizio usa davvero (le legge dai moduli esterni)
        new GeneratePackageJsonPlugin({ name, version, description, author, license, scripts: { start: scripts['script:dist:start'] } }),
    ],
    resolve: { extensions: ['.tsx', '.ts', '.js'] },
    node: { global: false, __filename: false, __dirname: false },
    output: { filename: 'index.js', path: path.join(ROOT, 'dist') },
    target: 'node',
};

module.exports = [frontend, backend];
